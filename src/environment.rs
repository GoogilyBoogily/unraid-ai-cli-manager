use crate::domain::{Context, ManagerError, Result, Settings};
use serde::Serialize;
use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone)]
pub(crate) struct Environment {
    root: PathBuf,
    pub(crate) fixture: bool,
}

impl Environment {
    pub(crate) fn from_process() -> Result<Self> {
        if let Some(root) = std::env::var_os("AI_CLI_MANAGER_TEST_ROOT") {
            #[cfg(feature = "test-support")]
            return Self::fixture(Path::new(&root));
            #[cfg(not(feature = "test-support"))]
            {
                let _ = root;
                return Err(ManagerError::Rejected(
                    "fixture_override_forbidden",
                    "this binary does not support fixture roots",
                ));
            }
        }
        if !rustix::process::geteuid().is_root() {
            return Err(ManagerError::Rejected(
                "root_required",
                "run the manager as root on Unraid",
            ));
        }
        let version =
            fs::read_to_string("/etc/unraid-version").context("cannot read Unraid version")?;
        if !supported_unraid(&version) {
            return Err(ManagerError::Rejected(
                "unsupported_unraid",
                "requires Unraid 7.3.4 or later in major version 7",
            ));
        }
        Ok(Self {
            root: PathBuf::from("/"),
            fixture: false,
        })
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn fixture(root: &Path) -> Result<Self> {
        let canonical = root.canonicalize().context("cannot resolve fixture root")?;
        if canonical != root
            || !canonical.starts_with("/tmp")
            || canonical == Path::new("/tmp")
            || fs::read_to_string(root.join(".ai-cli-manager-fixture"))
                .context("missing fixture sentinel")?
                != "ai-cli-manager-fixture-v1\n"
        {
            return Err(ManagerError::Rejected(
                "unsafe_fixture",
                "fixture root must be a marked real directory beneath /tmp",
            ));
        }
        Ok(Self {
            root: canonical,
            fixture: true,
        })
    }

    pub(crate) fn path(&self, absolute: impl AsRef<Path>) -> PathBuf {
        let relative = absolute
            .as_ref()
            .strip_prefix("/")
            .expect("manager paths are absolute constants");
        self.root.join(relative)
    }

    pub(crate) fn program(&self, name: &str) -> Result<PathBuf> {
        let allowed = [
            "rsync",
            "curl",
            "mountpoint",
            "logger",
            "notify",
            "update_cron",
            "bash",
            "sh",
        ]
        .contains(&name)
            || (cfg!(feature = "test-support") && self.fixture && name == "probe");
        if !allowed {
            return Err(ManagerError::Rejected(
                "invalid_program",
                "program is not allowlisted",
            ));
        }
        if self.fixture {
            let path = self.root.join("bin").join(name);
            if !path.is_file() {
                return Err(ManagerError::Rejected(
                    "missing_dependency",
                    "fixture program is missing",
                ));
            }
            return Ok(path);
        }
        let locations = if name == "notify" {
            vec![PathBuf::from("/usr/local/emhttp/webGui/scripts/notify")]
        } else {
            ["/usr/local/sbin", "/usr/sbin", "/usr/bin", "/sbin", "/bin"]
                .iter()
                .map(|directory| Path::new(directory).join(name))
                .collect()
        };
        locations
            .into_iter()
            .find(|path| {
                path.is_file()
                    && fs::metadata(path)
                        .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
            })
            .ok_or(ManagerError::Rejected(
                "missing_dependency",
                "required system program is missing",
            ))
    }

    pub(crate) fn storage_available(&self) -> Result<bool> {
        if self.fixture {
            return Ok(
                self.path("/.storage-mounted").is_file() && self.path("/mnt/user/appdata").is_dir()
            );
        }
        let mounted = Command::new(self.program("mountpoint")?)
            .args(["-q", "/mnt/user"])
            .status()
            .context("cannot check user-share mount")?;
        Ok(mounted.success() && self.path("/mnt/user/appdata").is_dir())
    }

    pub(crate) fn settings(&self) -> Result<Settings> {
        match fs::read(self.path("/boot/config/plugins/ai-cli-manager/settings.json")) {
            Ok(bytes) => {
                let settings: Settings = serde_json::from_slice(&bytes)?;
                settings.validate()?;
                Ok(settings)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(source) => Err(ManagerError::Io {
                context: "cannot read manager settings",
                source,
            }),
        }
    }
}

pub(crate) fn supported_unraid(contents: &str) -> bool {
    let version = contents
        .trim()
        .strip_prefix("version=")
        .unwrap_or(contents.trim())
        .trim_matches(['\'', '"']);
    if version.contains('-') {
        return false;
    }
    let numbers: Option<Vec<u32>> = version.split('.').map(|part| part.parse().ok()).collect();
    numbers.is_some_and(|numbers| {
        numbers.len() == 3 && numbers[0] == 7 && (numbers[1], numbers[2]) >= (3, 4)
    })
}

pub(crate) fn private_directory(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(ManagerError::Rejected(
            "unsafe_path",
            "manager control directory cannot be a symlink",
        ));
    }
    fs::create_dir_all(path).context("cannot create private directory")?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .context("cannot protect private directory")
}

pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .ok_or(ManagerError::Rejected("unsafe_path", "file has no parent"))?;
    private_directory(parent)?;
    replace_json(path, value)
}

pub(crate) fn replace_json(path: &Path, value: &impl Serialize) -> Result<()> {
    replace_bytes(path, &serde_json::to_vec(value)?)
}

pub(crate) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    private_directory(
        path.parent()
            .ok_or(ManagerError::Rejected("unsafe_path", "file has no parent"))?,
    )?;
    replace_bytes(path, bytes)
}

pub(crate) fn replace_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or(ManagerError::Rejected("unsafe_path", "file has no parent"))?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).context("cannot create private temporary file")?;
    temporary
        .write_all(bytes)
        .context("cannot write private temporary file")?;
    temporary
        .as_file()
        .sync_all()
        .context("cannot sync temporary file")?;
    temporary.persist(path).map_err(|error| ManagerError::Io {
        context: "cannot publish JSON file",
        source: error.error,
    })?;
    fs::File::open(parent)
        .context("cannot open publication directory")?
        .sync_all()
        .context("cannot sync publication directory")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_bounds_exclude_unraid_eight() {
        for version in ["7.3.3", "8.0.0", "8.0.0-rc.1", "garbage"] {
            assert!(!supported_unraid(version));
        }
        for version in ["version=\"7.3.4\"", "7.4.0"] {
            assert!(supported_unraid(version));
        }
    }
}
