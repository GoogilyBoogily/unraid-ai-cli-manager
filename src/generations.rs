use crate::{
    domain::{Context, ManagerError, OperationOutcome, Result, Tool},
    environment::{Environment, private_directory, replace_json, write_json},
    tools::{self, PackageSelection},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, symlink},
    },
    path::{Component, Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) struct StorageLease {
    directory: File,
    anchor: PathBuf,
}
impl StorageLease {
    pub(crate) fn acquire(environment: &Environment, tool: Tool, create: bool) -> Result<Self> {
        if !environment.storage_available()? {
            return Err(ManagerError::Rejected(
                "waiting_for_storage",
                "user shares are not mounted",
            ));
        }
        let expected_mount = if environment.fixture {
            None
        } else {
            let mounts = fs::read_to_string("/proc/self/mountinfo")
                .context("cannot inspect share mount identity")?;
            Some(
                mounts
                    .lines()
                    .find_map(|line| {
                        let fields: Vec<_> = line.split_whitespace().collect();
                        (fields.get(4) == Some(&"/mnt/user")).then(|| fields[0].to_owned())
                    })
                    .ok_or(ManagerError::Rejected(
                        "waiting_for_storage",
                        "user share mount is no longer present",
                    ))?,
            )
        };
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::NOFOLLOW).bits() as i32,
            )
            .open(environment.path("/mnt/user/appdata"))
            .context("cannot open mounted appdata directory")?;
        if let Some(expected) = expected_mount {
            let information =
                fs::read_to_string(format!("/proc/self/fdinfo/{}", directory.as_raw_fd()))
                    .context("cannot inspect appdata mount handle")?;
            let mounted = information
                .lines()
                .find_map(|line| line.strip_prefix("mnt_id:").map(str::trim));
            if mounted != Some(expected.as_str()) {
                return Err(ManagerError::Rejected(
                    "storage_changed",
                    "appdata filesystem changed during acquisition",
                ));
            }
        }
        let appdata_anchor = PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            directory.as_raw_fd()
        ));
        let tool_root = appdata_anchor.join(tool.legacy_name());
        if create {
            match fs::create_dir(&tool_root) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(ManagerError::Io {
                        context: "cannot create mounted persistence root",
                        source,
                    });
                }
            }
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::NOFOLLOW).bits() as i32,
            )
            .open(tool_root)
            .context("cannot pin persistence root")?;
        let metadata = rustix::fs::fstat(&directory)
            .map_err(io::Error::from)
            .context("cannot verify persistence ownership")?;
        if metadata.st_uid != rustix::process::geteuid().as_raw() {
            return Err(ManagerError::Rejected(
                "unsafe_path",
                "persistence root must belong to the manager user",
            ));
        }
        if create {
            rustix::fs::fchmod(&directory, rustix::fs::Mode::RWXU)
                .map_err(io::Error::from)
                .context("cannot protect persistence root")?;
        }
        let anchor = PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            directory.as_raw_fd()
        ));
        Ok(Self { directory, anchor })
    }
    pub(crate) fn root(&self) -> PathBuf {
        self.anchor.clone()
    }
    fn available_bytes(&self) -> Result<u64> {
        let status = rustix::fs::fstatvfs(&self.directory)
            .map_err(io::Error::from)
            .context("cannot inspect appdata capacity")?;
        Ok(status.f_bavail.saturating_mul(status.f_frsize))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(crate) manager: String,
    pub(crate) format_version: u32,
    pub(crate) tool: Tool,
    pub(crate) generation_identifier: String,
    pub(crate) predecessor_identifier: Option<String>,
    pub(crate) created_at: u64,
    pub(crate) package: PackageSelection,
}
pub(crate) struct CommittedGeneration {
    _storage: StorageLease,
    pub(crate) path: PathBuf,
    pub(crate) manifest: Manifest,
}

pub(crate) fn timestamp() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ManagerError::Rejected("clock_failure", "system clock precedes epoch"))?
        .as_secs())
}

fn valid_identifier(identifier: &str) -> bool {
    identifier.starts_with("generation-")
        && identifier.len() <= 100
        && identifier
            .bytes()
            .all(|character| character.is_ascii_alphanumeric() || character == b'-')
}

fn selected(
    environment: &Environment,
    storage: &StorageLease,
    tool: Tool,
    check_databases: bool,
) -> Result<Option<(PathBuf, Manifest)>> {
    let root = storage.root();
    let target = match fs::read_link(root.join("current")) {
        Ok(target) => target,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ManagerError::Io {
                context: "cannot read current generation selector",
                source,
            });
        }
    };
    let components: Vec<_> = target.components().collect();
    if components.len() != 2 || components[0] != Component::Normal("generations".as_ref()) {
        return Err(ManagerError::Rejected(
            "invalid_generation",
            "generation selector is unsafe",
        ));
    }
    let identifier = components[1]
        .as_os_str()
        .to_str()
        .ok_or(ManagerError::Rejected(
            "invalid_generation",
            "generation identifier is invalid",
        ))?;
    if !valid_identifier(identifier) {
        return Err(ManagerError::Rejected(
            "invalid_generation",
            "generation identifier is invalid",
        ));
    }
    let path = root.join(&target);
    let canonical = path
        .canonicalize()
        .context("selected generation is missing")?;
    let generations = root.join("generations");
    if fs::symlink_metadata(&generations)
        .context("generation directory is missing")?
        .file_type()
        .is_symlink()
        || !canonical.starts_with(
            generations
                .canonicalize()
                .context("cannot resolve generation directory")?,
        )
        || fs::symlink_metadata(&path)
            .context("selected generation is missing")?
            .file_type()
            .is_symlink()
    {
        return Err(ManagerError::Rejected(
            "invalid_generation",
            "generation escapes its managed directory",
        ));
    }
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(path.join("manifest.json")).context("generation manifest is missing")?,
    )?;
    if manifest.manager != "ai-cli-manager"
        || manifest.format_version != 1
        || manifest.tool != tool
        || manifest.generation_identifier != identifier
    {
        return Err(ManagerError::Rejected(
            "invalid_generation",
            "generation manifest does not match its selector",
        ));
    }
    tools::validate_payload(
        &path.join("root"),
        &environment.path("/root"),
        tool,
        &manifest.package,
    )?;
    if check_databases && tool == Tool::Codex {
        for database in tools::database_paths(&path.join("root/.codex"))? {
            crate::sqlite::validate_database(&database)?;
        }
    }
    Ok(Some((path, manifest)))
}

pub(crate) fn read_current(environment: &Environment, tool: Tool) -> Result<CommittedGeneration> {
    let storage = StorageLease::acquire(environment, tool, false)?;
    let (path, manifest) = selected(environment, &storage, tool, true)?.ok_or(
        ManagerError::Rejected("missing_generation", "no committed generation exists"),
    )?;
    Ok(CommittedGeneration {
        _storage: storage,
        path,
        manifest,
    })
}

fn synchronize_tree(root: &Path) -> Result<()> {
    let mut paths = tools::walk(root)?;
    paths.push(root.to_owned());
    for path in paths
        .iter()
        .filter(|path| path.is_file() && !path.is_symlink())
    {
        File::open(path)
            .context("cannot open snapshot file for sync")?
            .sync_all()
            .context("cannot sync snapshot file")?;
    }
    for path in paths
        .iter()
        .rev()
        .filter(|path| path.is_dir() && !path.is_symlink())
    {
        File::open(path)
            .context("cannot open snapshot directory for sync")?
            .sync_all()
            .context("cannot sync snapshot directory")?;
    }
    Ok(())
}

pub(crate) fn capture(
    environment: &Environment,
    tool: Tool,
    deadline: Instant,
) -> Result<CommittedGeneration> {
    if !environment.path(tool.marker()).is_file() {
        return Err(ManagerError::Rejected(
            "initialization_required",
            "restore marker is missing; refusing backup",
        ));
    }
    capture_validated_live(environment, tool, deadline)
}

pub(crate) fn capture_initial(
    environment: &Environment,
    tool: Tool,
    deadline: Instant,
) -> Result<CommittedGeneration> {
    if !environment.path(tool.marker()).is_file() && tools::active_tool(environment, tool)? {
        return Err(ManagerError::Rejected(
            "needs_attention",
            "active sessions have no valid initialization marker",
        ));
    }
    capture_validated_live(environment, tool, deadline)
}

fn capture_validated_live(
    environment: &Environment,
    tool: Tool,
    deadline: Instant,
) -> Result<CommittedGeneration> {
    let storage = StorageLease::acquire(environment, tool, true)?;
    let selection = tools::inspect_live(environment, tool)?;
    let live = environment.path(tool.live());
    let mut required = tools::walk(&live)?
        .iter()
        .try_fold(0u64, |total, path| -> Result<u64> {
            let metadata = fs::symlink_metadata(path).context("cannot estimate backup capacity")?;
            Ok(total.saturating_add(if metadata.is_file() {
                metadata.len()
            } else {
                0
            }))
        })?;
    if tool == Tool::Claude {
        for path in tools::walk(&environment.path("/root/.local/share/claude/versions"))? {
            let metadata =
                fs::symlink_metadata(path).context("cannot estimate package capacity")?;
            if metadata.is_file() {
                required = required.saturating_add(metadata.len());
            }
        }
    }
    let _ = selection;
    if storage.available_bytes()? < required.saturating_add(64 * 1024 * 1024) {
        return Err(ManagerError::Rejected(
            "insufficient_capacity",
            "insufficient capacity for a complete new generation",
        ));
    }
    let root = storage.root();
    private_directory(&root.join("generations"))?;
    private_directory(&root.join(".staging"))?;
    let previous =
        selected(environment, &storage, tool, true)?.map(|entry| entry.1.generation_identifier);
    let staging = tempfile::Builder::new()
        .prefix("generation-")
        .tempdir_in(root.join(".staging"))
        .context("cannot create snapshot staging directory")?;
    let identifier = staging
        .path()
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(ManagerError::Rejected(
            "invalid_generation",
            "cannot name generation",
        ))?
        .to_owned();
    let payload = staging.path().join("root");
    write_json(
        &staging.path().join("owner.json"),
        &json!({"manager":"ai-cli-manager", "generation_identifier":identifier}),
    )?;
    private_directory(&payload)?;
    let package = tools::capture(environment, tool, &payload, deadline)?;
    let manifest = Manifest {
        manager: "ai-cli-manager".to_owned(),
        format_version: 1,
        tool,
        generation_identifier: identifier.clone(),
        predecessor_identifier: previous,
        created_at: timestamp()?,
        package,
    };
    write_json(&staging.path().join("manifest.json"), &manifest)?;
    synchronize_tree(staging.path())?;
    if Instant::now() >= deadline {
        return Err(ManagerError::Rejected(
            "deadline_exceeded",
            "snapshot publication deadline exceeded",
        ));
    }
    let staging_path = staging.keep();
    let published = root.join("generations").join(&identifier);
    fs::rename(&staging_path, &published).context("cannot publish complete generation")?;
    File::open(root.join("generations"))
        .context("cannot open generation directory")?
        .sync_all()
        .context("cannot sync published generation")?;
    let selector = root.join(format!(".selector-{identifier}"));
    symlink(Path::new("generations").join(&identifier), &selector)
        .context("cannot stage generation selector")?;
    fs::rename(selector, root.join("current")).context("cannot select completed generation")?;
    File::open(&root)
        .context("cannot open persistence root")?
        .sync_all()
        .context("cannot sync current generation selector")?;
    replace_json(
        &root.join("last-success.json"),
        &json!({"generation_identifier": identifier, "completed_at": timestamp()?}),
    )?;
    prune(&root, &manifest)?;
    Ok(CommittedGeneration {
        _storage: storage,
        path: published,
        manifest,
    })
}

fn prune(root: &Path, current: &Manifest) -> Result<()> {
    for entry in
        fs::read_dir(root.join("generations")).context("cannot inspect retained generations")?
    {
        let entry = entry.context("cannot read retained generation")?;
        let name = entry.file_name();
        let name = name.to_str().ok_or(ManagerError::Rejected(
            "invalid_generation",
            "invalid retained generation name",
        ))?;
        if !valid_identifier(name)
            || name == current.generation_identifier
            || current.predecessor_identifier.as_deref() == Some(name)
        {
            continue;
        }
        if entry
            .file_type()
            .context("cannot inspect retained generation type")?
            .is_symlink()
        {
            return Err(ManagerError::Rejected(
                "invalid_generation",
                "retained generation cannot be a symlink",
            ));
        }
        let manifest: Manifest = serde_json::from_slice(
            &fs::read(entry.path().join("manifest.json"))
                .context("old generation ownership is unproven")?,
        )?;
        if manifest.manager != "ai-cli-manager"
            || manifest.tool != current.tool
            || manifest.generation_identifier != name
        {
            return Err(ManagerError::Rejected(
                "invalid_generation",
                "refusing to prune unowned data",
            ));
        }
        fs::remove_dir_all(entry.path()).context("cannot prune superseded manager generation")?;
    }
    for entry in fs::read_dir(root.join(".staging")).context("cannot inspect abandoned staging")? {
        let entry = entry.context("cannot inspect abandoned staging entry")?;
        if !entry
            .file_type()
            .context("cannot inspect staging type")?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !valid_identifier(name) {
            continue;
        }
        let owner_path = entry.path().join("owner.json");
        let bytes = match fs::read(owner_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(ManagerError::Io {
                    context: "cannot verify abandoned staging ownership",
                    source,
                });
            }
        };
        let owner: Value = serde_json::from_slice(&bytes)?;
        if owner["manager"] == "ai-cli-manager" && owner["generation_identifier"] == name {
            fs::remove_dir_all(entry.path()).context("cannot remove abandoned manager staging")?;
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    #[test]
    fn renamed_share_path_does_not_redirect_storage_writes_to_rootfs() {
        let root = tempfile::tempdir_in("/tmp").expect("fixture");
        fs::write(
            root.path().join(".ai-cli-manager-fixture"),
            "ai-cli-manager-fixture-v1\n",
        )
        .expect("sentinel");
        fs::write(root.path().join(".storage-mounted"), "mounted").expect("mount");
        fs::create_dir_all(root.path().join("mnt/user/appdata")).expect("share");
        let environment = Environment::fixture(root.path()).expect("environment");
        let storage = StorageLease::acquire(&environment, Tool::Claude, true).expect("lease");
        fs::rename(
            root.path().join("mnt/user"),
            root.path().join("mnt/prior-share"),
        )
        .expect("mount path removed");
        fs::create_dir(root.path().join("mnt/user")).expect("rootfs directory");
        private_directory(&storage.root().join("new-generation")).expect("anchored write");
        assert!(!root.path().join("mnt/user/appdata").exists());
        assert!(
            root.path()
                .join("mnt/prior-share/appdata/claude-code")
                .is_dir()
        );
        assert!(
            root.path()
                .join("mnt/prior-share/appdata/claude-code/new-generation")
                .is_dir()
        );
    }
}

pub(crate) fn restore(
    environment: &Environment,
    tool: Tool,
    deadline: Instant,
) -> Result<OperationOutcome> {
    if environment.path(tool.marker()).exists() {
        tools::inspect_live(environment, tool)?;
        return Ok(OperationOutcome::unchanged(
            json!({"ram_authoritative":true}),
        ));
    }
    if tools::active_tool(environment, tool)? {
        return Err(ManagerError::Rejected(
            "needs_attention",
            "active tool sessions prevent restore",
        ));
    }
    let generation = read_current(environment, tool)?;
    let journal_path =
        environment.path(format!("/run/ai-cli-manager/restore-{}.json", tool.name()));
    let recovering = match fs::read(&journal_path) {
        Ok(bytes) => {
            serde_json::from_slice::<Value>(&bytes)?["generation_identifier"]
                == generation.manifest.generation_identifier
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(source) => {
            return Err(ManagerError::Io {
                context: "cannot inspect restore journal",
                source,
            });
        }
    };
    let live = environment.path(tool.live());
    let nonempty = if live.exists() {
        fs::read_dir(&live)
            .context("cannot inspect live state")?
            .next()
            .is_some()
    } else {
        false
    };
    if !recovering
        && (nonempty
            || environment.path(tool.launcher()).symlink_metadata().is_ok()
            || (tool == Tool::Claude
                && environment
                    .path("/root/.claude.json")
                    .symlink_metadata()
                    .is_ok()))
    {
        return Err(ManagerError::Rejected(
            "needs_attention",
            "conflicting live state has no valid initialization; refusing restore",
        ));
    }
    private_directory(&environment.path("/root/.ai-cli-manager"))?;
    let temporary = tempfile::tempdir_in(environment.path("/root/.ai-cli-manager"))
        .context("cannot stage RAM restore")?;
    let candidate = temporary.path().join("root");
    tools::transfer(
        environment,
        &generation.path.join("root"),
        &candidate,
        &[],
        deadline,
    )?;
    tools::validate_payload(
        &candidate,
        &environment.path("/root"),
        tool,
        &generation.manifest.package,
    )?;
    write_json(
        &journal_path,
        &json!({"generation_identifier":generation.manifest.generation_identifier}),
    )?;
    if recovering {
        merge_recovery(&candidate, &environment.path("/root"), tool)?;
    }
    match tool {
        Tool::Claude => {
            tools::transfer(
                environment,
                &candidate.join(".claude"),
                &live,
                &[],
                deadline,
            )?;
            if candidate.join(".claude.json").symlink_metadata().is_ok() {
                tools::copy_entry(
                    &candidate.join(".claude.json"),
                    &environment.path("/root/.claude.json"),
                )?;
            }
            tools::transfer(
                environment,
                &candidate.join(".local/share/claude/versions"),
                &environment.path("/root/.local/share/claude/versions"),
                &[],
                deadline,
            )?;
        }
        Tool::Codex => {
            tools::transfer(environment, &candidate.join(".codex"), &live, &[], deadline)?
        }
    }
    tools::protect_state(&live)?;
    tools::install_launchers(environment, tool, &generation.manifest.package)?;
    tools::inspect_live(environment, tool)?;
    let policy = crate::configuration::plan_policy(environment, tool, false)?;
    crate::configuration::apply_policy(environment, &policy)?;
    crate::configuration::record_policy(environment, tool)?;
    fs::write(environment.path(tool.marker()), "ai-cli-manager restored\n")
        .context("cannot publish boot restore marker")?;
    Ok(OperationOutcome::persisted(
        json!({"generation_identifier":generation.manifest.generation_identifier}),
    ))
}

fn merge_recovery(candidate: &Path, live_root: &Path, tool: Tool) -> Result<()> {
    let json_paths: &[&str] = match tool {
        Tool::Claude => &[".claude/settings.json", ".claude.json"],
        Tool::Codex => &[],
    };
    for relative in json_paths {
        let live = live_root.join(relative);
        let persisted = candidate.join(relative);
        if live.is_file() && !live.is_symlink() && persisted.is_file() && !persisted.is_symlink() {
            let merged = tools::merge_json(
                serde_json::from_slice(
                    &fs::read(&persisted).context("cannot read recovery configuration")?,
                )?,
                serde_json::from_slice(
                    &fs::read(&live).context("cannot read live recovery configuration")?,
                )?,
            );
            write_json(&persisted, &merged)?;
        }
    }
    let relative = match tool {
        Tool::Claude => ".claude/history.jsonl",
        Tool::Codex => ".codex/history.jsonl",
    };
    if live_root.join(relative).is_file() && candidate.join(relative).is_file() {
        let merged = tools::merge_history(
            &fs::read_to_string(candidate.join(relative))
                .context("cannot read persisted history")?,
            &fs::read_to_string(live_root.join(relative)).context("cannot read live history")?,
        );
        fs::write(candidate.join(relative), merged).context("cannot stage merged history")?;
    }
    Ok(())
}

pub(crate) fn status_summary(environment: &Environment, tool: Tool) -> Result<Value> {
    let root = environment.path(tool.persistent());
    if !root.exists() {
        return Ok(json!({"version":null,"last_backup":null,"last_update":null}));
    }
    let storage = StorageLease::acquire(environment, tool, false)?;
    let generation = selected(environment, &storage, tool, false)?;
    let mut receipts = Vec::new();
    for name in ["last-success.json", "last-update.json"] {
        let receipt = match crate::configuration::read_optional(&storage.root().join(name))? {
            Some(bytes) => {
                let value: Value = serde_json::from_slice(&bytes)?;
                value["completed_at"].as_u64()
            }
            None => None,
        };
        receipts.push(receipt);
    }
    Ok(
        json!({"version":generation.map(|(_,manifest)|manifest.package.version),"last_backup":receipts[0],"last_update":receipts[1]}),
    )
}
