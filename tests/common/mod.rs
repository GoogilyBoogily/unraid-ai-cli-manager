#![allow(dead_code)]
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};

pub struct Fixture {
    pub directory: tempfile::TempDir,
}
impl Fixture {
    pub fn manager_programs(&self) {
        for program in ["notify", "logger", "update_cron"] {
            self.executable(&format!("bin/{program}"), "#!/bin/sh\nexit 0\n");
        }
        self.executable("bin/rsync", "#!/bin/sh\nexec /usr/bin/rsync \"$@\"\n");
        self.executable("bin/bash", "#!/bin/sh\nexec /bin/bash \"$@\"\n");
        self.executable("bin/sh", "#!/bin/sh\nexec /bin/sh \"$@\"\n");
        self.write("var/log/plugins/ai-cli-manager.plg", "fixture registration");
    }
    pub fn wait(&self, receipt: &Value) -> Value {
        use std::{
            thread,
            time::{Duration, Instant},
        };
        let identifier = receipt["job_identifier"]
            .as_str()
            .or_else(|| receipt["jobs"][0]["receipt"]["job_identifier"].as_str())
            .expect("job receipt");
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let job = self.json(&["__internal", "job", "--identifier", identifier]);
            if ["succeeded", "failed", "cancelled"]
                .contains(&job["state"].as_str().expect("job state"))
            {
                return job;
            }
            assert!(Instant::now() < deadline, "job did not complete: {job}");
            thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn claude(&self, version: &str) {
        use std::os::unix::fs::symlink;
        self.write("root/.claude/settings.json", "{\"userSetting\":42}");
        self.executable(
            &format!("root/.local/share/claude/versions/{version}"),
            &format!("#!/bin/sh\necho '{version} (Claude Code)'\n"),
        );
        fs::create_dir_all(self.root().join("root/.local/bin")).expect("launcher directory");
        symlink(
            format!("../share/claude/versions/{version}"),
            self.root().join("root/.local/bin/claude"),
        )
        .expect("launcher");
        self.write("root/.claude-persist-restored", "legacy restore marker");
    }
    pub fn new() -> Self {
        let directory = tempfile::tempdir_in("/tmp").expect("fixture directory");
        let fixture = Self { directory };
        fixture.write(".ai-cli-manager-fixture", "ai-cli-manager-fixture-v1\n");
        fixture.write("boot/config/plugins/ai-cli-manager/settings.json", &json!({"backup_interval_minutes":15,"claude":{"enabled":true,"initialized":true,"automatic_updates":true},"codex":{"enabled":true,"initialized":true,"automatic_updates":true}}).to_string());
        fixture
    }
    pub fn root(&self) -> &Path {
        self.directory.path()
    }
    pub fn write(&self, relative: &str, contents: &str) {
        let path = self.root().join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create fixture parent");
        fs::write(path, contents).expect("write fixture file");
    }
    pub fn executable(&self, relative: &str, contents: &str) {
        self.write(relative, contents);
        fs::set_permissions(
            self.root().join(relative),
            fs::Permissions::from_mode(0o700),
        )
        .expect("executable mode");
    }
    pub fn command(&self, arguments: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ai-cli-manager"))
            .env("AI_CLI_MANAGER_TEST_ROOT", self.root())
            .args(arguments)
            .output()
            .expect("run manager")
    }
    pub fn json(&self, arguments: &[&str]) -> Value {
        let output = self.command(arguments);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("JSON output")
    }
    pub fn mount(&self) {
        fs::create_dir_all(self.root().join("mnt/user/appdata")).expect("fixture shares");
        self.write(".storage-mounted", "mounted");
    }
}
