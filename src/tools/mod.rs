use crate::{
    domain::{Context, ManagerError, Result, Tool},
    environment::{Environment, private_directory},
    process, sqlite,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Component, Path, PathBuf},
    time::Instant,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PackageSelection {
    pub(crate) version: String,
    pub(crate) relative_executable: PathBuf,
}

pub(crate) fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).context("cannot enumerate managed state")? {
            let entry = entry.context("cannot inspect managed directory entry")?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).context("cannot inspect managed file")?;
            if metadata.is_dir() {
                pending.push(path.clone());
            }
            paths.push(path);
        }
    }
    Ok(paths)
}

fn executable(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path).context("required executable is missing")?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(ManagerError::Rejected(
            "invalid_package",
            "required package executable is invalid",
        ));
    }
    Ok(())
}

pub(crate) fn inspect_live(environment: &Environment, tool: Tool) -> Result<PackageSelection> {
    let root = environment.path("/root");
    let live = environment.path(tool.live());
    if !fs::symlink_metadata(&live)
        .context("live source is missing")?
        .is_dir()
    {
        return Err(ManagerError::Rejected(
            "missing_source",
            "live source must be a real RAM directory",
        ));
    }
    let selected = environment
        .path(tool.launcher())
        .canonicalize()
        .context("tool launcher is missing or broken")?;
    let relative_executable = selected
        .strip_prefix(&root)
        .map_err(|_| {
            ManagerError::Rejected(
                "invalid_package",
                "launcher resolves outside managed RAM state",
            )
        })?
        .to_owned();
    let version = match tool {
        Tool::Claude => selected
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(ManagerError::Rejected(
                "invalid_package",
                "Claude version filename is invalid",
            ))?
            .to_owned(),
        Tool::Codex => {
            let package =
                selected
                    .parent()
                    .and_then(Path::parent)
                    .ok_or(ManagerError::Rejected(
                        "invalid_package",
                        "Codex package root is missing",
                    ))?;
            let metadata: serde_json::Value = serde_json::from_slice(
                &fs::read(package.join("codex-package.json"))
                    .context("Codex package metadata is missing")?,
            )?;
            metadata["version"]
                .as_str()
                .ok_or(ManagerError::Rejected(
                    "invalid_package",
                    "Codex package version is missing",
                ))?
                .to_owned()
        }
    };
    let selection = PackageSelection {
        version,
        relative_executable,
    };
    validate_payload(&root, tool, &selection)?;
    Ok(selection)
}

pub(crate) fn validate_payload(
    root: &Path,
    tool: Tool,
    selection: &PackageSelection,
) -> Result<()> {
    let configuration_paths: &[&str] = match tool {
        Tool::Claude => &[
            ".claude/settings.json",
            ".claude.json",
            ".claude/.credentials.json",
        ],
        Tool::Codex => &[".codex/config.toml", ".codex/auth.json"],
    };
    for relative in configuration_paths {
        let path = root.join(relative);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(ManagerError::Io {
                    context: "cannot inspect required configuration",
                    source,
                });
            }
        };
        if metadata.file_type().is_symlink() {
            if !path
                .try_exists()
                .context("cannot inspect external configuration target")?
            {
                return Err(ManagerError::Rejected(
                    "missing_external_target",
                    "required external configuration target is missing",
                ));
            }
            continue;
        }
        let contents = fs::read_to_string(path).context("cannot read required configuration")?;
        if relative.ends_with(".toml") {
            let _: toml_edit::DocumentMut = contents.parse()?;
        } else {
            let _: serde_json::Value = serde_json::from_str(&contents)?;
        }
    }
    if selection.relative_executable.is_absolute()
        || selection
            .relative_executable
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(ManagerError::Rejected(
            "invalid_package",
            "unsafe package selection",
        ));
    }
    let package_root = match tool {
        Tool::Claude => root.join(".local/share/claude/versions"),
        Tool::Codex => root.join(".codex/packages/standalone/releases"),
    };
    let package_root = package_root
        .canonicalize()
        .context("package releases are missing")?;
    let binary = root
        .join(&selection.relative_executable)
        .canonicalize()
        .context("selected executable is missing")?;
    if !binary.starts_with(&package_root) {
        return Err(ManagerError::Rejected(
            "invalid_package",
            "package selection escapes managed releases",
        ));
    }
    executable(&binary)?;
    if tool == Tool::Codex {
        let release = binary
            .parent()
            .and_then(Path::parent)
            .ok_or(ManagerError::Rejected(
                "invalid_package",
                "invalid Codex package layout",
            ))?;
        let metadata: serde_json::Value = serde_json::from_slice(
            &fs::read(release.join("codex-package.json"))
                .context("Codex package manifest is missing")?,
        )?;
        if metadata["layoutVersion"] != 1
            || metadata["entrypoint"] != "bin/codex"
            || metadata["version"] != selection.version
        {
            return Err(ManagerError::Rejected(
                "invalid_package",
                "unsupported Codex package layout or version",
            ));
        }
        for relative in [
            "bin/codex-code-mode-host",
            "codex-path/rg",
            "codex-resources/bwrap",
        ] {
            let helper = release
                .join(relative)
                .canonicalize()
                .context("required Codex helper is missing")?;
            if !helper.starts_with(release) {
                return Err(ManagerError::Rejected(
                    "invalid_package",
                    "Codex helper escapes its package",
                ));
            }
            executable(&helper)?;
        }
        if let Some(server) = metadata["execServer"].as_str() {
            if server != "bin/codex-exec-server" {
                return Err(ManagerError::Rejected(
                    "invalid_package",
                    "unsupported Codex exec server layout",
                ));
            }
            executable(&release.join(server))?;
        }
    }
    for path in walk(&package_root)? {
        if fs::symlink_metadata(&path)
            .context("cannot inspect package link")?
            .file_type()
            .is_symlink()
            && !path
                .canonicalize()
                .context("package has a broken link")?
                .starts_with(&package_root)
        {
            return Err(ManagerError::Rejected(
                "invalid_package",
                "package link resolves outside the cached package",
            ));
        }
    }
    Ok(())
}

fn trailing_slash(path: &Path) -> OsString {
    let mut value = path.as_os_str().to_os_string();
    value.push("/");
    value
}
pub(crate) fn transfer(
    environment: &Environment,
    source: &Path,
    destination: &Path,
    excludes: &[&str],
    deadline: Instant,
) -> Result<()> {
    private_directory(destination)?;
    let mut arguments: Vec<OsString> = ["-a", "--no-devices", "--no-specials"]
        .iter()
        .map(OsString::from)
        .collect();
    arguments.extend(
        excludes
            .iter()
            .map(|exclude| OsString::from(format!("--exclude={exclude}"))),
    );
    arguments.extend([
        OsString::from("--"),
        trailing_slash(source),
        trailing_slash(destination),
    ]);
    process::run_checked(environment, "rsync", &arguments, deadline)?;
    Ok(())
}

pub(crate) fn database_paths(root: &Path) -> Result<Vec<PathBuf>> {
    use std::io::Read;
    let mut databases = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).context("cannot enumerate database sources")? {
            let entry = entry.context("cannot inspect database source")?;
            let path = entry.path();
            if ["packages", "log", "tmp", ".tmp"]
                .iter()
                .any(|name| path == root.join(name))
            {
                continue;
            }
            let metadata =
                fs::symlink_metadata(&path).context("cannot inspect database source type")?;
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let mut header = [0; 16];
            let length = fs::File::open(&path)
                .context("cannot open database candidate")?
                .read(&mut header)
                .context("cannot inspect database signature")?;
            let recognized_name = path.extension().is_some_and(|extension| {
                ["sqlite", "sqlite3", "db"]
                    .iter()
                    .any(|suffix| extension == *suffix)
            });
            if recognized_name || (length == 16 && &header == b"SQLite format 3\0") {
                databases.push(path);
            }
        }
    }
    Ok(databases)
}

pub(crate) fn capture(
    environment: &Environment,
    tool: Tool,
    destination: &Path,
    deadline: Instant,
) -> Result<PackageSelection> {
    let selection = inspect_live(environment, tool)?;
    let root = environment.path("/root");
    match tool {
        Tool::Claude => {
            transfer(
                environment,
                &root.join(".claude"),
                &destination.join(".claude"),
                &[
                    "/shell-snapshots/",
                    "/session-env/",
                    "/cache/",
                    "/downloads/",
                    "/paste-cache/",
                ],
                deadline,
            )?;
            let configuration = root.join(".claude.json");
            if fs::symlink_metadata(&configuration).is_ok() {
                copy_entry(&configuration, &destination.join(".claude.json"))?;
            }
            transfer(
                environment,
                &root.join(".local/share/claude/versions"),
                &destination.join(".local/share/claude/versions"),
                &[],
                deadline,
            )?;
        }
        Tool::Codex => {
            let databases = database_paths(&root.join(".codex"))?;
            let mut exclusions: Vec<String> = [
                "/log/",
                "/tmp/",
                "/.tmp/",
                "*.sqlite",
                "*.sqlite3",
                "*.db",
                "*-wal",
                "*-shm",
                "*-journal",
                "*.pid",
                "*.sock",
                "install.lock",
                "install.lock.d",
            ]
            .iter()
            .map(|value| (*value).to_owned())
            .collect();
            for database in &databases {
                let relative = database
                    .strip_prefix(root.join(".codex"))
                    .expect("database is rooted")
                    .to_str()
                    .ok_or(ManagerError::Rejected(
                        "invalid_state",
                        "database path is not UTF-8",
                    ))?;
                let mut escaped = String::from("/");
                for character in relative.chars() {
                    if ['\\', '[', ']', '*', '?'].contains(&character) {
                        escaped.push('\\');
                    }
                    escaped.push(character);
                }
                exclusions.push(escaped);
            }
            transfer(
                environment,
                &root.join(".codex"),
                &destination.join(".codex"),
                &exclusions.iter().map(String::as_str).collect::<Vec<_>>(),
                deadline,
            )?;
            for source in databases {
                let relative = source
                    .strip_prefix(&root)
                    .expect("database within live root");
                sqlite::backup_database(&source, &destination.join(relative), deadline)?;
            }
        }
    }
    normalize_package_links(&root, destination, tool)?;
    protect_state(destination)?;
    validate_payload(destination, tool, &selection)?;
    if inspect_live(environment, tool)? != selection {
        return Err(ManagerError::Rejected(
            "package_changed",
            "package selection changed during backup",
        ));
    }
    Ok(selection)
}

fn relative_link(parent: &Path, target: &Path) -> PathBuf {
    let parent: Vec<_> = parent.components().collect();
    let target: Vec<_> = target.components().collect();
    let common = parent
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = PathBuf::new();
    for _ in common..parent.len() {
        relative.push("..");
    }
    for component in &target[common..] {
        relative.push(component.as_os_str());
    }
    relative
}

fn normalize_package_links(source_root: &Path, destination: &Path, tool: Tool) -> Result<()> {
    let relative = match tool {
        Tool::Claude => ".local/share/claude/versions",
        Tool::Codex => ".codex/packages",
    };
    let source_package = source_root
        .join(relative)
        .canonicalize()
        .context("source package is missing")?;
    for path in walk(&destination.join(relative))? {
        if !fs::symlink_metadata(&path)
            .context("cannot inspect package selection")?
            .file_type()
            .is_symlink()
        {
            continue;
        }
        let source = source_root.join(path.strip_prefix(destination).expect("staging path rooted"));
        let selected = source
            .canonicalize()
            .context("package has a dangling source link")?;
        if !selected.starts_with(&source_package) {
            return Err(ManagerError::Rejected(
                "invalid_package",
                "package source link escapes its package tree",
            ));
        }
        let staged_target = destination.join(selected.strip_prefix(source_root).map_err(|_| {
            ManagerError::Rejected("invalid_package", "package target is outside live root")
        })?);
        fs::remove_file(&path).context("cannot normalize staged package link")?;
        symlink(
            relative_link(path.parent().expect("link parent"), &staged_target),
            path,
        )
        .context("cannot create staged package link")?;
    }
    Ok(())
}

pub(crate) fn protect_state(root: &Path) -> Result<()> {
    for path in walk(root)? {
        let metadata = fs::symlink_metadata(&path).context("cannot inspect state permissions")?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let sensitive = path.extension().is_some_and(|extension| {
            ["json", "jsonl", "toml", "sqlite", "sqlite3", "db"]
                .iter()
                .any(|suffix| extension == *suffix)
        });
        let mode =
            if metadata.is_dir() || (!sensitive && metadata.permissions().mode() & 0o111 != 0) {
                0o700
            } else {
                0o600
            };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .context("cannot protect sensitive state")?;
    }
    Ok(())
}

pub(crate) fn copy_entry(source: &Path, destination: &Path) -> Result<()> {
    private_directory(destination.parent().ok_or(ManagerError::Rejected(
        "unsafe_path",
        "state file has no parent",
    ))?)?;
    let metadata = fs::symlink_metadata(source).context("required state file is missing")?;
    if metadata.file_type().is_symlink() {
        if destination.symlink_metadata().is_ok() {
            fs::remove_file(destination).context("cannot replace managed link")?;
        }
        symlink(
            fs::read_link(source).context("cannot read state link")?,
            destination,
        )
        .context("cannot preserve state link")?;
    } else {
        let temporary =
            tempfile::NamedTempFile::new_in(destination.parent().expect("parent checked"))
                .context("cannot stage state file")?;
        fs::copy(source, temporary.path()).context("cannot copy state file")?;
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o600))
            .context("cannot protect state file")?;
        temporary
            .as_file()
            .sync_all()
            .context("cannot sync state file")?;
        temporary
            .persist(destination)
            .map_err(|error| ManagerError::Io {
                context: "cannot publish state file",
                source: error.error,
            })?;
    }
    Ok(())
}

pub(crate) fn install_launchers(
    environment: &Environment,
    tool: Tool,
    selection: &PackageSelection,
) -> Result<()> {
    private_directory(&environment.path("/root/.local/bin"))?;
    for path in [
        environment.path(tool.launcher()),
        environment.path(format!("/usr/local/bin/{}", tool.name())),
    ] {
        fs::create_dir_all(path.parent().expect("launcher parent"))
            .context("cannot create launcher directory")?;
        let temporary = tempfile::Builder::new()
            .prefix(".launcher-")
            .tempdir_in(path.parent().expect("launcher parent"))
            .context("cannot stage launcher")?;
        let staged = temporary.path().join("link");
        symlink(
            environment
                .path("/root")
                .join(&selection.relative_executable),
            &staged,
        )
        .context("cannot stage launcher selection")?;
        fs::rename(staged, path).context("cannot select tool launcher")?;
    }
    Ok(())
}

pub(crate) fn merge_json(
    persisted: serde_json::Value,
    live: serde_json::Value,
) -> serde_json::Value {
    match (persisted, live) {
        (serde_json::Value::Object(mut persisted), serde_json::Value::Object(live)) => {
            for (key, value) in live {
                let merged = match persisted.remove(&key) {
                    Some(previous) => merge_json(previous, value),
                    None => value,
                };
                persisted.insert(key, merged);
            }
            serde_json::Value::Object(persisted)
        }
        (_, live) => live,
    }
}

pub(crate) fn merge_history(persisted: &str, live: &str) -> String {
    let mut seen = HashSet::new();
    let mut result = String::new();
    for line in persisted.lines().chain(live.lines()) {
        if seen.insert(line) {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

pub(crate) fn active_tool(environment: &Environment, tool: Tool) -> Result<bool> {
    let prefix = match tool {
        Tool::Claude => environment.path("/root/.local/share/claude"),
        Tool::Codex => environment.path("/root/.codex/packages"),
    };
    for entry in fs::read_dir("/proc").context("cannot inspect active tool processes")? {
        let entry = entry.context("cannot inspect process entry")?;
        if entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
            .is_none()
        {
            continue;
        }
        if !entry
            .metadata()
            .is_ok_and(|metadata| metadata.uid() == rustix::process::geteuid().as_raw())
        {
            continue;
        }
        match fs::read_link(entry.path().join("exe")) {
            Ok(executable) if executable.starts_with(&prefix) => return Ok(true),
            Ok(_) => {}
            Err(error)
                if [
                    std::io::ErrorKind::NotFound,
                    std::io::ErrorKind::PermissionDenied,
                ]
                .contains(&error.kind()) => {}
            Err(source) => {
                return Err(ManagerError::Io {
                    context: "cannot inspect active executable",
                    source,
                });
            }
        }
    }
    Ok(false)
}
