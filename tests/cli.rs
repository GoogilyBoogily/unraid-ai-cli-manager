use std::process::Command;

#[cfg(feature = "test-support")]
mod common;

#[test]
fn help_describes_the_supported_operations() {
    let output = Command::new(env!("CARGO_BIN_EXE_ai-cli-manager"))
        .arg("--help").output().expect("run manager");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    for operation in ["status", "restore", "backup", "install", "update"] {
        assert!(help.contains(operation), "missing {operation}");
    }
}

#[test]
fn invalid_request_cannot_start_work() {
    let output = Command::new(env!("CARGO_BIN_EXE_ai-cli-manager"))
        .args(["backup", "--tool", "codex;touch /tmp/manager-injection"])
        .output().expect("run manager");
    assert!(!output.status.success());
}

#[cfg(feature = "test-support")]
#[test]
fn status_waits_without_creating_share_paths() {
    let fixture = common::Fixture::new();
    let status = fixture.json(&["status", "--tool", "all", "--json"]);
    assert_eq!(status["tools"]["claude"]["readiness"], "waiting_for_storage");
    assert!(!fixture.root().join("mnt/user").exists());
}

#[test]
fn fixture_root_refuses_root_or_symlink_escape() {
    let output = Command::new(env!("CARGO_BIN_EXE_ai-cli-manager"))
        .env("AI_CLI_MANAGER_TEST_ROOT", "/")
        .args(["status", "--json"]).output().expect("run manager");
    assert!(!output.status.success());
}

