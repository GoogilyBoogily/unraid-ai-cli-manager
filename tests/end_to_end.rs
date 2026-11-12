#![cfg(feature = "test-support")]
mod common;
use std::{
    fs, thread,
    time::{Duration, Instant},
};
#[test]
fn explicit_adoption_backup_cold_start_and_disable_preserve_state() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("1.0.0");
    fixture.write("boot/config/plugins/ai-cli-manager/settings.json",r#"{"backup_interval_minutes":15,"claude":{"enabled":false,"initialized":false,"automatic_updates":true},"codex":{"enabled":false,"initialized":false,"automatic_updates":true}}"#);
    fixture.json(&["__internal", "start"]);
    assert!(!fixture.root().join("mnt/user/appdata/claude-code").exists());
    assert_eq!(
        fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "claude"]))["state"],
        "succeeded"
    );
    fixture.write("root/.claude/history.jsonl", "old\nnew\n");
    assert_eq!(
        fixture.wait(&fixture.json(&["backup", "--tool", "claude"]))["state"],
        "succeeded"
    );
    for (source, destination) in [
        ("root/.claude", "saved-claude"),
        ("root/.local", "saved-local"),
        ("root/.claude-persist-restored", "saved-marker"),
    ] {
        fs::rename(
            fixture.root().join(source),
            fixture.root().join(destination),
        )
        .unwrap();
    }
    fixture.json(&["__internal", "event", "stopping"]);
    fixture.json(&["__internal", "event", "disks_mounted"]);
    let deadline = Instant::now() + Duration::from_secs(4);
    while !fixture
        .root()
        .join("root/.claude-persist-restored")
        .exists()
    {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        fs::read_to_string(fixture.root().join("root/.claude/history.jsonl")).unwrap(),
        "old\nnew\n"
    );
    assert!(fixture.root().join("root/.local/bin/claude").exists());
    let status = fixture.json(&["status", "--tool", "claude"]);
    assert_eq!(
        fixture.wait(&status["tools"]["claude"]["job"])["state"],
        "succeeded"
    );
    let settings_path = fixture
        .root()
        .join("boot/config/plugins/ai-cli-manager/settings.json");
    let mut settings: serde_json::Value =
        serde_json::from_slice(&fs::read(&settings_path).unwrap()).unwrap();
    settings["claude"]["enabled"] = false.into();
    fs::write(settings_path, settings.to_string()).unwrap();
    assert_eq!(
        fixture.json(&[
            "__internal",
            "tick",
            "--operation",
            "backup",
            "--tool",
            "claude"
        ])["skipped"],
        "disabled"
    );
    assert!(
        fixture
            .root()
            .join("mnt/user/appdata/claude-code/current")
            .exists()
    );
}
