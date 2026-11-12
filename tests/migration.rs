#![cfg(feature = "test-support")]
mod common;
use serde_json::{Value, json};
use std::fs;

fn prepared() -> common::Fixture {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.manager_programs();
    fixture.claude("1.0.0");
    fixture.write("boot/config/plugins/ai-cli-manager/settings.json",&json!({"backup_interval_minutes":15,"claude":{"enabled":false,"initialized":false,"automatic_updates":true},"codex":{"enabled":false,"initialized":false,"automatic_updates":true}}).to_string());
    fixture.write(
        "boot/config/claude-code/claude-persist.sh",
        include_str!("fixtures/legacy/claude-persist.sh"),
    );
    let mut schedules = serde_json::Map::new();
    for tool in ["claude-code", "codex"] {
        for (action, frequency) in [
            ("restore", "start"),
            ("backup", "custom"),
            ("backup-on-stop", "stop"),
        ] {
            let path = format!("/boot/config/plugins/user.scripts/scripts/{tool}-{action}/script");
            schedules.insert(path.clone(),json!({"script":path,"frequency":frequency,"id":format!("{tool}-{action}"),"custom":"*/15 * * * *"}));
            if tool == "claude-code" {
                fixture.write(
                    path.trim_start_matches('/'),
                    &format!(
                        "#!/bin/bash\n/boot/config/claude-code/claude-persist.sh {}\n",
                        if action == "restore" {
                            "restore"
                        } else {
                            "backup"
                        }
                    ),
                );
            }
        }
    }
    schedules.insert("unrelated".to_owned(),json!({"script":"/unrelated","frequency":"daily","id":"other","custom":"","extra":"preserve"}));
    for path in [
        "boot/config/plugins/user.scripts/schedule.json",
        "tmp/user.scripts/schedule.json",
    ] {
        fixture.write(path, &Value::Object(schedules.clone()).to_string());
    }
    fixture.write("boot/config/plugins/user.scripts/customSchedule.cron","# Generated cron schedule for user.scripts\n*/15 * * * * /usr/local/emhttp/plugins/user.scripts/startCustom.php /boot/config/plugins/user.scripts/scripts/claude-code-backup/script > /dev/null 2>&1\n*/15 * * * * /usr/local/emhttp/plugins/user.scripts/startCustom.php /boot/config/plugins/user.scripts/scripts/codex-backup/script > /dev/null 2>&1\n");
    fixture.write(
        "mnt/user/appdata/claude-code/.claude/legacy.txt",
        "legacy retained",
    );
    fixture
}

#[test]
fn takeover_disables_only_selected_tool_in_both_copies_and_cron() {
    let fixture = prepared();
    let job = fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "claude"]));
    assert_eq!(job["state"], "succeeded", "{job}");
    for path in [
        "boot/config/plugins/user.scripts/schedule.json",
        "tmp/user.scripts/schedule.json",
    ] {
        let schedule: Value =
            serde_json::from_slice(&fs::read(fixture.root().join(path)).expect("schedule"))
                .expect("JSON");
        for action in ["restore", "backup", "backup-on-stop"] {
            assert_eq!(
                schedule[format!(
                    "/boot/config/plugins/user.scripts/scripts/claude-code-{action}/script"
                )]["frequency"],
                "disabled"
            );
        }
        assert_eq!(
            schedule["/boot/config/plugins/user.scripts/scripts/codex-backup/script"]["frequency"],
            "custom"
        );
        assert_eq!(schedule["unrelated"]["extra"], "preserve");
    }
    let cron = fs::read_to_string(
        fixture
            .root()
            .join("boot/config/plugins/user.scripts/customSchedule.cron"),
    )
    .expect("cron");
    assert!(!cron.contains("claude-code-backup/script"));
    assert!(cron.contains("codex-backup/script"));
    assert_eq!(
        fs::read_to_string(
            fixture
                .root()
                .join("mnt/user/appdata/claude-code/legacy-migration-backup/.claude/legacy.txt")
        )
        .expect("legacy backup"),
        "legacy retained"
    );
}

#[test]
fn unknown_legacy_source_refuses_takeover_before_schedule_changes() {
    let fixture = prepared();
    let path = fixture
        .root()
        .join("boot/config/plugins/user.scripts/schedule.json");
    let before = fs::read(&path).expect("before");
    fixture.write(
        "boot/config/claude-code/claude-persist.sh",
        "#!/bin/bash\necho unknown\n",
    );
    let job = fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "claude"]));
    assert_eq!(job["state"], "failed");
    assert_eq!(job["error_code"], "legacy_mismatch");
    assert_eq!(fs::read(path).expect("after"), before);
}

#[test]
fn failed_cron_takeover_rolls_back_only_owned_entries_and_live_policy() {
    let fixture = prepared();
    let original = fs::read(
        fixture
            .root()
            .join("boot/config/plugins/user.scripts/schedule.json"),
    )
    .expect("schedule");
    fixture.executable("bin/update_cron","#!/bin/sh\nif [ ! -e \"$AI_CLI_MANAGER_TEST_ROOT/failed-once\" ]; then touch \"$AI_CLI_MANAGER_TEST_ROOT/failed-once\"; exit 1; fi\n");
    let job = fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "claude"]));
    assert_eq!(job["state"], "failed", "{job}");
    let after: Value = serde_json::from_slice(
        &fs::read(
            fixture
                .root()
                .join("boot/config/plugins/user.scripts/schedule.json"),
        )
        .expect("schedule"),
    )
    .expect("JSON");
    assert_eq!(
        after,
        serde_json::from_slice::<Value>(&original).expect("before")
    );
    let settings: Value = serde_json::from_slice(
        &fs::read(fixture.root().join("root/.claude/settings.json")).expect("config"),
    )
    .expect("JSON");
    assert!(settings.get("env").is_none());
}

#[test]
fn rollback_preserves_concurrent_unrelated_edits_and_multiple_owned_cron_lines() {
    let fixture = prepared();
    let cron = fixture
        .root()
        .join("boot/config/plugins/user.scripts/customSchedule.cron");
    let mut contents = fs::read_to_string(&cron).unwrap();
    contents.push_str("0 * * * * /usr/local/emhttp/plugins/user.scripts/startCustom.php /boot/config/plugins/user.scripts/scripts/claude-code-restore/script > /dev/null 2>&1\n");
    fs::write(&cron, &contents).unwrap();
    fixture.executable("bin/update_cron", "#!/bin/sh\nif [ ! -e \"$AI_CLI_MANAGER_TEST_ROOT/failed-once\" ]; then\n touch \"$AI_CLI_MANAGER_TEST_ROOT/failed-once\"\n /usr/bin/python3 -c 'import json,os,pathlib; p=pathlib.Path(os.environ[\"AI_CLI_MANAGER_TEST_ROOT\"])/\"boot/config/plugins/user.scripts/schedule.json\"; d=json.loads(p.read_text()); d[\"unrelated\"][\"extra\"]=\"concurrent\"; p.write_text(json.dumps(d))'\n exit 1\nfi\n");
    let job = fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "claude"]));
    assert_eq!(job["state"], "failed");
    assert_eq!(job["error_code"], "command_failed", "{job}");
    let schedule: Value = serde_json::from_slice(
        &fs::read(
            fixture
                .root()
                .join("boot/config/plugins/user.scripts/schedule.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(schedule["unrelated"]["extra"], "concurrent");
    assert_eq!(fs::read_to_string(cron).unwrap(), contents);
}

#[test]
fn interrupted_disabled_phase_rolls_back_before_a_second_takeover_attempt() {
    let fixture = prepared();
    assert_eq!(
        fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "claude"]))["state"],
        "succeeded"
    );
    let path = fixture
        .root()
        .join("boot/config/plugins/ai-cli-manager/migration-claude.json");
    let mut journal: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    journal["phase"] = "Disabled".into();
    fs::write(&path, journal.to_string()).unwrap();
    let result = fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "claude"]));
    assert_eq!(result["state"], "failed");
    assert_eq!(result["error_code"], "needs_attention", "{result}");
    let schedule: Value = serde_json::from_slice(
        &fs::read(
            fixture
                .root()
                .join("boot/config/plugins/user.scripts/schedule.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        schedule["/boot/config/plugins/user.scripts/scripts/claude-code-backup/script"]["frequency"],
        "custom"
    );
    assert!(
        fixture
            .root()
            .join("mnt/user/appdata/claude-code/.claude/legacy.txt")
            .exists()
    );
    let cron = fs::read_to_string(
        fixture
            .root()
            .join("boot/config/plugins/ai-cli-manager/schedule.cron"),
    )
    .unwrap();
    assert!(!cron.contains("--tool claude"));
    assert_eq!(
        fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "claude"]))["state"],
        "succeeded"
    );
}

#[test]
fn initialization_waits_for_the_existing_legacy_lock() {
    let fixture = prepared();
    fs::create_dir_all(fixture.root().join("var/run")).unwrap();
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(fixture.root().join("var/run/claude-persist.lock"))
        .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
    let receipt = fixture.json(&["__internal", "initialize", "--tool", "claude"]);
    let job = fixture.json(&[
        "__internal",
        "job",
        "--identifier",
        receipt["job_identifier"].as_str().unwrap(),
    ]);
    assert_eq!(job["state"], "queued");
    drop(lock);
    assert_eq!(fixture.wait(&receipt)["state"], "succeeded");
}

#[test]
fn codex_takeover_preserves_an_already_committed_claude_takeover() {
    use std::os::unix::fs::symlink;
    let fixture = prepared();
    assert_eq!(
        fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "claude"]))["state"],
        "succeeded"
    );
    fixture.write(
        "boot/config/codex/codex-persist.sh",
        include_str!("fixtures/legacy/codex-persist.sh"),
    );
    for action in ["restore", "backup", "backup-on-stop"] {
        fixture.write(
            &format!("boot/config/plugins/user.scripts/scripts/codex-{action}/script"),
            &format!(
                "#!/bin/sh\n/boot/config/codex/codex-persist.sh {}\n",
                if action == "restore" {
                    "restore"
                } else {
                    "backup"
                }
            ),
        );
    }
    let release = "root/.codex/packages/standalone/releases/1.0.0-linux";
    fixture.write(
        &format!("{release}/codex-package.json"),
        r#"{"layoutVersion":1,"version":"1.0.0","entrypoint":"bin/codex"}"#,
    );
    for helper in [
        "bin/codex",
        "bin/codex-code-mode-host",
        "codex-path/rg",
        "codex-resources/bwrap",
    ] {
        fixture.executable(
            &format!("{release}/{helper}"),
            "#!/bin/sh\necho codex-cli 1.0.0\n",
        );
    }
    symlink(
        fixture.root().join(release),
        fixture
            .root()
            .join("root/.codex/packages/standalone/current"),
    )
    .unwrap();
    symlink(
        fixture.root().join(format!("{release}/bin/codex")),
        fixture.root().join("root/.local/bin/codex"),
    )
    .unwrap();
    fixture.write("root/.codex-persist-restored", "legacy marker");
    fixture.write(
        "root/.codex/config.toml",
        "approval_policy = \"on-request\"\n",
    );
    fixture.write("mnt/user/appdata/codex/.codex/old.txt", "retain old codex");
    assert_eq!(
        fixture.wait(&fixture.json(&["__internal", "initialize", "--tool", "codex"]))["state"],
        "succeeded"
    );
    for path in [
        "boot/config/plugins/user.scripts/schedule.json",
        "tmp/user.scripts/schedule.json",
    ] {
        let schedule: Value =
            serde_json::from_slice(&fs::read(fixture.root().join(path)).unwrap()).unwrap();
        for tool in ["claude-code", "codex"] {
            for action in ["restore", "backup", "backup-on-stop"] {
                assert_eq!(
                    schedule[format!(
                        "/boot/config/plugins/user.scripts/scripts/{tool}-{action}/script"
                    )]["frequency"],
                    "disabled"
                );
            }
        }
        assert_eq!(schedule["unrelated"]["extra"], "preserve");
    }
    assert!(
        fixture
            .root()
            .join("mnt/user/appdata/claude-code/current")
            .exists()
    );
    assert!(
        fixture
            .root()
            .join("mnt/user/appdata/codex/legacy-migration-backup/.codex/old.txt")
            .exists()
    );
}
