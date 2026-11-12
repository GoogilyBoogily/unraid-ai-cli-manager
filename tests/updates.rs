#![cfg(feature = "test-support")]
mod common;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

fn settings(fixture: &common::Fixture, automatic: bool) {
    fixture.write("boot/config/plugins/ai-cli-manager/settings.json", &json!({"backup_interval_minutes":15,"claude":{"enabled":true,"initialized":true,"automatic_updates":automatic},"codex":{"enabled":false,"initialized":false,"automatic_updates":true}}).to_string());
}
fn installer(fixture: &common::Fixture, version: &str) {
    fixture.write(
        "boot/config/plugins/ai-cli-manager/test-approved-versions.json",
        &json!({"claude":{"version":version,"server_verified":true},"codex":null}).to_string(),
    );
    fixture.write("vendor-installer.sh", "#!/bin/bash\nset -eu\nmkdir -p \"$HOME/.local/share/claude/versions\" \"$HOME/.local/bin\" \"$HOME/.claude\"\nprintf '#!/bin/sh\\necho \"%s (Claude Code)\"\\n' \"$1\" > \"$HOME/.local/share/claude/versions/$1\"\nchmod 700 \"$HOME/.local/share/claude/versions/$1\"\nln -s \"$HOME/.local/share/claude/versions/$1\" \"$HOME/.local/bin/claude\"\n");
    fixture.executable("bin/curl", "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = '--output' ]; then cp \"$AI_CLI_MANAGER_TEST_ROOT/vendor-installer.sh\" \"$2\"; exit 0; fi; shift; done\nexit 22\n");
}
#[test]
fn manual_update_works_with_automatic_updates_off_and_persists() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("1.0.0");
    settings(&fixture, false);
    installer(&fixture, "1.1.0");
    let job = fixture.wait(&fixture.json(&["update", "--tool", "claude"]));
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(job["installation"], "succeeded");
    assert_eq!(job["persistence"], "succeeded");
    assert!(
        fixture
            .root()
            .join("root/.local/share/claude/versions/1.0.0")
            .exists()
    );
    assert_eq!(
        fs::read_to_string(fixture.root().join("root/.claude/settings.json"))
            .expect("settings")
            .parse::<Value>()
            .expect("JSON")["userSetting"],
        42
    );
}
#[test]
fn same_version_skips_download_and_newer_manual_version_is_not_downgraded() {
    for (installed, target) in [("1.0.0", "1.0.0"), ("2.0.0", "1.0.0")] {
        let fixture = common::Fixture::new();
        fixture.mount();
        fixture.manager_programs();
        fixture.claude(installed);
        settings(&fixture, true);
        installer(&fixture, target);
        fixture.executable("bin/curl", "#!/bin/sh\nexit 99\n");
        let job = fixture.wait(&fixture.json(&["update", "--tool", "claude"]));
        assert_eq!(job["state"], "succeeded", "{job}");
        assert_eq!(job["installation"], "not_attempted");
        assert!(
            fixture
                .root()
                .join(format!("root/.local/share/claude/versions/{installed}"))
                .exists()
        );
    }
}
#[test]
fn initialize_is_explicit_and_preserves_existing_user_configuration() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("1.0.0");
    fixture.write("boot/config/plugins/ai-cli-manager/settings.json", &json!({"backup_interval_minutes":15,"claude":{"enabled":false,"initialized":false,"automatic_updates":true},"codex":{"enabled":false,"initialized":false,"automatic_updates":true}}).to_string());
    let job = fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "claude"]));
    assert_eq!(job["state"], "succeeded", "{job}");
    let configuration: Value = serde_json::from_slice(
        &fs::read(fixture.root().join("root/.claude/settings.json")).expect("config"),
    )
    .expect("JSON");
    assert_eq!(configuration["userSetting"], 42);
    assert_eq!(configuration["env"]["DISABLE_AUTOUPDATER"], "1");
    assert!(configuration["env"].get("DISABLE_UPDATES").is_none());
    let saved: Value = serde_json::from_slice(
        &fs::read(
            fixture
                .root()
                .join("boot/config/plugins/ai-cli-manager/settings.json"),
        )
        .expect("settings"),
    )
    .expect("JSON");
    assert_eq!(saved["claude"]["initialized"], true);
    assert_eq!(saved["claude"]["enabled"], true);
    assert_eq!(saved["codex"]["enabled"], false);
}
#[test]
fn invalid_settings_reject_before_writing() {
    let fixture = common::Fixture::new();
    fixture.manager_programs();
    let path = fixture
        .root()
        .join("boot/config/plugins/ai-cli-manager/settings.json");
    let before = fs::read(&path).expect("settings");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ai-cli-manager"))
        .env("AI_CLI_MANAGER_TEST_ROOT", fixture.root())
        .args(["__internal", "settings"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("manager");
    child
        .stdin
        .take()
        .expect("input")
        .write_all(b"{\"backup_interval_minutes\":7}")
        .expect("write settings");
    assert!(!child.wait_with_output().expect("response").status.success());
    assert_eq!(fs::read(path).expect("settings"), before);
}

#[test]
fn failed_candidate_retains_selection_and_policy_and_no_update_receipt() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("1.0.0");
    settings(&fixture, true);
    installer(&fixture, "1.1.0");
    fixture.write("vendor-installer.sh", "#!/bin/sh\nexit 17\n");
    let before = fs::read(fixture.root().join("root/.claude/settings.json")).unwrap();
    let job = fixture.wait(&fixture.json(&["update", "--tool", "claude"]));
    assert_eq!(job["state"], "failed");
    assert_eq!(job["error_code"], "installer_failed");
    assert!(
        fs::canonicalize(fixture.root().join("root/.local/bin/claude"))
            .unwrap()
            .ends_with("1.0.0")
    );
    assert_eq!(
        fs::read(fixture.root().join("root/.claude/settings.json")).unwrap(),
        before
    );
    assert!(
        !fixture
            .root()
            .join("mnt/user/appdata/claude-code/last-update.json")
            .exists()
    );
}

#[test]
fn successful_installation_reports_later_persistence_failure_separately() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("1.0.0");
    settings(&fixture, true);
    installer(&fixture, "1.1.0");
    fixture.executable("bin/rsync","#!/bin/sh\nif [ -e \"$AI_CLI_MANAGER_TEST_ROOT/root/.local/share/claude/versions/1.1.0\" ]; then exit 28; fi\nexec /usr/bin/rsync \"$@\"\n");
    let job = fixture.wait(&fixture.json(&["update", "--tool", "claude"]));
    assert_eq!(job["state"], "failed", "{job}");
    assert_eq!(job["installation"], "succeeded");
    assert_eq!(job["persistence"], "failed");
    assert!(
        fs::canonicalize(fixture.root().join("root/.local/bin/claude"))
            .unwrap()
            .ends_with("1.1.0")
    );
    let status = fixture.json(&["status", "--tool", "claude"]);
    assert_eq!(status["tools"]["claude"]["versions"]["cached"], "1.0.0");
    assert!(
        !fixture
            .root()
            .join("mnt/user/appdata/claude-code/last-update.json")
            .exists()
    );
}

fn codex_installer(fixture: &common::Fixture, version: &str) {
    fixture.write(
        "boot/config/plugins/ai-cli-manager/test-approved-versions.json",
        &json!({"claude":null,"codex":{"version":version,"server_verified":true}}).to_string(),
    );
    fixture.write("vendor-installer.sh",r#"#!/bin/sh
set -eu
version=$2
release="$HOME/.codex/packages/standalone/releases/$version-x86_64-unknown-linux-musl"
mkdir -p "$release/bin" "$release/codex-path" "$release/codex-resources" "$HOME/.local/bin"
printf '{"layoutVersion":1,"version":"%s","entrypoint":"bin/codex"}' "$version" > "$release/codex-package.json"
for helper in bin/codex bin/codex-code-mode-host codex-path/rg codex-resources/bwrap; do
  printf '#!/bin/sh\necho codex-cli %s\n' "$version" > "$release/$helper"
  chmod 700 "$release/$helper"
done
ln -s "$release" "$HOME/.codex/packages/standalone/current"
ln -s "$release/bin/codex" "$HOME/.local/bin/codex"
printf '%s\n' "$version" > "$HOME/.codex/packages/standalone/auto-update-version"
mkdir -p "$HOME/.codex/packages/app-server-daemon/releases/candidate"
printf replaced > "$HOME/.codex/packages/app-server-daemon/releases/candidate/daemon"
"#);
    fixture.executable("bin/curl","#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = '--output' ]; then cp \"$AI_CLI_MANAGER_TEST_ROOT/vendor-installer.sh\" \"$2\"; exit 0; fi; shift; done\nexit 22\n");
}

#[test]
fn fresh_codex_requires_opt_in_for_host_policy_and_never_copies_updater_metadata_or_daemon() {
    for consent in [false, true] {
        let fixture = common::Fixture::new();
        fixture.mount();
        fixture.manager_programs();
        fixture.write("boot/config/plugins/ai-cli-manager/settings.json",r#"{"backup_interval_minutes":15,"claude":{"enabled":false,"initialized":false,"automatic_updates":true},"codex":{"enabled":false,"initialized":false,"automatic_updates":true}}"#);
        codex_installer(&fixture, "1.0.0");
        let mut arguments = vec!["__internal", "initialize", "--tool", "codex"];
        if consent {
            arguments.push("--host-execution-opt-in");
        }
        let job = fixture.wait(&fixture.json(&arguments));
        assert_eq!(job["state"], "succeeded", "{job}");
        let configuration: toml_edit::DocumentMut =
            fs::read_to_string(fixture.root().join("root/.codex/config.toml"))
                .unwrap()
                .parse()
                .unwrap();
        assert_eq!(
            configuration
                .get("sandbox_mode")
                .and_then(|item| item.as_str()),
            if consent {
                Some("danger-full-access")
            } else {
                None
            }
        );
        assert_eq!(
            configuration
                .get("approval_policy")
                .and_then(|item| item.as_str()),
            if consent { Some("on-request") } else { None }
        );
        assert!(
            !fixture
                .root()
                .join("root/.codex/packages/standalone/auto-update-version")
                .exists()
        );
        assert!(
            !fixture
                .root()
                .join("root/.codex/packages/app-server-daemon/releases/candidate")
                .exists()
        );
        fixture.write(
            "root/.codex/packages/standalone/auto-update-version",
            "1.0.0\n",
        );
        fixture.write(
            "root/.codex/packages/app-server-daemon/releases/old/daemon",
            "keep-running-daemon",
        );
        codex_installer(&fixture, "1.1.0");
        assert_eq!(
            fixture.wait(&fixture.json(&["update", "--tool", "codex"]))["state"],
            "succeeded"
        );
        assert_eq!(
            fs::read(
                fixture
                    .root()
                    .join("root/.codex/packages/standalone/auto-update-version")
            )
            .unwrap(),
            b""
        );
        assert_eq!(
            fs::read_to_string(
                fixture
                    .root()
                    .join("root/.codex/packages/app-server-daemon/releases/old/daemon")
            )
            .unwrap(),
            "keep-running-daemon"
        );
        assert!(
            !fixture
                .root()
                .join("root/.codex/packages/app-server-daemon/releases/candidate")
                .exists()
        );
    }
}

#[test]
fn newer_manual_version_cannot_inherit_an_older_targets_policy_verification() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("2.0.0");
    settings(&fixture, true);
    installer(&fixture, "1.0.0");
    assert_eq!(
        fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "claude"]))["state"],
        "succeeded"
    );
    let status = fixture.json(&["status", "--tool", "claude"]);
    assert_eq!(
        status["tools"]["claude"]["native_update_policy"]["verified_on_server"],
        false
    );
}
