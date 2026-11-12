#![cfg(feature = "test-support")]
mod common;
use std::{
    fs, thread,
    time::{Duration, Instant},
};

#[test]
fn shutdown_cancels_writer_rejects_work_and_preserves_installation() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("2.0.1");
    fixture.executable("bin/probe", "#!/bin/sh\nsetsid sh -c 'trap \"\" TERM; while :; do echo tick >> \"$AI_CLI_MANAGER_TEST_ROOT/writes\"; sleep 0.01; done' &\nwait\n");
    let receipt = fixture.json(&[
        "__internal",
        "probe",
        "--tool",
        "codex",
        "--milliseconds",
        "60000",
    ]);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !fixture.root().join("writes").exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    // Only the initialized Claude installation receives the final backup.
    let path = fixture
        .root()
        .join("boot/config/plugins/ai-cli-manager/settings.json");
    let mut settings: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    settings["codex"]["enabled"] = false.into();
    fs::write(path, settings.to_string()).unwrap();
    fixture.json(&["__internal", "event", "stopping"]);
    let blocked = fixture.command(&["backup", "--tool", "claude"]);
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("stopping"));
    fixture.json(&["__internal", "shutdown", "--budget-milliseconds", "2000"]);
    assert_eq!(fixture.wait(&receipt)["state"], "cancelled");
    let before = fs::metadata(fixture.root().join("writes")).unwrap().len();
    thread::sleep(Duration::from_millis(80));
    assert_eq!(
        fs::metadata(fixture.root().join("writes")).unwrap().len(),
        before
    );
    assert!(fixture.root().join("root/.local/bin/claude").exists());
    assert!(
        fixture
            .root()
            .join("mnt/user/appdata/claude-code/current")
            .exists()
    );
}

#[test]
fn stale_update_tick_obeys_off_and_plugin_hook_filters_registration() {
    let fixture = common::Fixture::new();
    fixture.manager_programs();
    let path = fixture
        .root()
        .join("boot/config/plugins/ai-cli-manager/settings.json");
    let mut settings: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    settings["claude"]["automatic_updates"] = false.into();
    fs::write(path, settings.to_string()).unwrap();
    assert_eq!(
        fixture.json(&[
            "__internal",
            "tick",
            "--operation",
            "update",
            "--tool",
            "claude"
        ])["skipped"],
        "disabled"
    );
    fixture.json(&[
        "__internal",
        "plugin-hook",
        "plugin",
        "install",
        "other.plg",
        "",
    ]);
    assert!(
        !fixture
            .root()
            .join("boot/config/plugins/ai-cli-manager/schedule.cron")
            .exists()
    );
    fixture.json(&[
        "__internal",
        "plugin-hook",
        "plugin",
        "install",
        "ai-cli-manager.plg",
        "",
    ]);
    let cron = fs::read_to_string(
        fixture
            .root()
            .join("boot/config/plugins/ai-cli-manager/schedule.cron"),
    )
    .unwrap();
    assert!(cron.contains("0 4 * * *"));
    assert!(
        !cron
            .lines()
            .any(|line| line.contains("--operation update --tool claude"))
    );
    assert!(cron.contains("--operation backup --tool claude"));
}

#[test]
fn status_reports_real_cached_version_and_safe_failure() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("2.0.1");
    fixture.write(
        "root/.claude/credentials.json",
        "{\"secret\":\"NEVER-STATUS-THIS\"}",
    );
    let job = fixture.json(&["backup", "--tool", "claude"]);
    assert_eq!(fixture.wait(&job)["state"], "succeeded");
    let status = fixture.json(&["status", "--tool", "claude", "--json"]);
    assert_eq!(status["tools"]["claude"]["versions"]["installed"], "2.0.1");
    assert_eq!(status["tools"]["claude"]["versions"]["cached"], "2.0.1");
    assert!(status["tools"]["claude"]["last_successful_backup"].is_number());
    assert_eq!(status["tools"]["claude"]["readiness"], "needs_attention"); // Not in approved catalog.
    assert!(!status.to_string().contains("NEVER-STATUS-THIS"));
}

#[test]
fn notifications_deduplicate_failures_and_reset_after_success() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("1.0.0");
    fixture.executable(
        "bin/notify",
        "#!/bin/sh\necho delivered >> \"$AI_CLI_MANAGER_TEST_ROOT/notifications\"\n",
    );
    fixture.executable("bin/rsync", "#!/bin/sh\nexit 23\n");
    for expected in [1, 1] {
        assert_eq!(
            fixture.wait(&fixture.json(&["backup", "--tool", "claude"]))["state"],
            "failed"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while !fixture
            .root()
            .join("run/ai-cli-manager/notification-claude.json")
            .exists()
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(30));
        assert_eq!(
            fs::read_to_string(fixture.root().join("notifications"))
                .unwrap()
                .lines()
                .count(),
            expected
        );
    }
    fixture.executable("bin/rsync", "#!/bin/sh\nexec /usr/bin/rsync \"$@\"\n");
    assert_eq!(
        fixture.wait(&fixture.json(&["backup", "--tool", "claude"]))["state"],
        "succeeded"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let state: serde_json::Value = serde_json::from_slice(
            &fs::read(
                fixture
                    .root()
                    .join("run/ai-cli-manager/notification-claude.json"),
            )
            .unwrap(),
        )
        .unwrap();
        if state["failure"].is_null() {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    fixture.executable("bin/rsync", "#!/bin/sh\nexit 23\n");
    assert_eq!(
        fixture.wait(&fixture.json(&["backup", "--tool", "claude"]))["state"],
        "failed"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if fs::read_to_string(fixture.root().join("notifications"))
            .unwrap()
            .lines()
            .count()
            == 2
        {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn uninstall_preserves_live_state_settings_and_legacy_jobs() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("1.0.0");
    fixture.write("boot/config/plugins/ai-cli-manager/settings.json",r#"{"backup_interval_minutes":15,"claude":{"enabled":true,"initialized":true,"automatic_updates":true},"codex":{"enabled":false,"initialized":false,"automatic_updates":true}}"#);
    fixture.write(
        "boot/config/plugins/user.scripts/schedule.json",
        "legacy schedules remain disabled",
    );
    let before = fs::read(
        fixture
            .root()
            .join("boot/config/plugins/ai-cli-manager/settings.json"),
    )
    .unwrap();
    assert_eq!(fixture.json(&["__internal", "uninstall"])["drained"], true);
    assert_eq!(
        fs::read(
            fixture
                .root()
                .join("boot/config/plugins/ai-cli-manager/settings.json")
        )
        .unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(
            fixture
                .root()
                .join("boot/config/plugins/user.scripts/schedule.json")
        )
        .unwrap(),
        "legacy schedules remain disabled"
    );
    assert!(fixture.root().join("root/.local/bin/claude").exists());
    assert!(
        fixture
            .root()
            .join("mnt/user/appdata/claude-code/current")
            .exists()
    );
    assert!(
        !fs::read_to_string(
            fixture
                .root()
                .join("boot/config/plugins/ai-cli-manager/schedule.cron")
        )
        .unwrap()
        .contains("--operation")
    );
}

#[test]
fn incomplete_migration_blocks_scheduling_and_reports_recovery() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("1.0.0");
    fixture.write("boot/config/plugins/ai-cli-manager/migration-claude.json",r#"{"format_version":1,"tool":"claude","phase":"Disabled","copies":[],"cron_lines":[],"legacy_entries":[],"policy_file":"policy-fixture.json","settings_before":{"enabled":false,"initialized":false,"automatic_updates":true}}"#);
    let result = fixture.command(&[
        "__internal",
        "tick",
        "--operation",
        "backup",
        "--tool",
        "claude",
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("migration_recovery_required"));
    fixture.json(&[
        "__internal",
        "plugin-hook",
        "plugin",
        "install",
        "ai-cli-manager.plg",
        "",
    ]);
    let cron = fs::read_to_string(
        fixture
            .root()
            .join("boot/config/plugins/ai-cli-manager/schedule.cron"),
    )
    .unwrap();
    assert!(!cron.contains("--tool claude"));
    let status = fixture.json(&["status", "--tool", "claude"]);
    assert!(
        status["tools"]["claude"]["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["code"] == "migration_recovery_required")
    );
}
