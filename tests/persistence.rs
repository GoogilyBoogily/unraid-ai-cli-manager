#![cfg(feature = "test-support")]
mod common;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    thread,
    time::{Duration, Instant},
};

fn prepared() -> common::Fixture {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.write("root/.claude/settings.json", "{\"answer\":42}");
    fixture.write("root/.claude.json", "{\"secret\":\"fixture-only-secret\"}");
    fixture.write("root/.claude-persist-restored", "restored");
    fixture.executable(
        "root/.local/share/claude/versions/1.0.0",
        "#!/bin/sh\necho '1.0.0 (Claude Code)'\n",
    );
    fs::create_dir_all(fixture.root().join("root/.local/bin")).expect("launcher directory");
    symlink(
        "../share/claude/versions/1.0.0",
        fixture.root().join("root/.local/bin/claude"),
    )
    .expect("launcher");
    fixture.executable("bin/rsync", "#!/bin/sh\nexec /usr/bin/rsync \"$@\"\n");
    fixture
}

fn backup(fixture: &common::Fixture) -> serde_json::Value {
    let receipt = fixture.json(&["backup", "--tool", "claude", "--json"]);
    let identifier = receipt["jobs"][0]["receipt"]["job_identifier"]
        .as_str()
        .expect("job identifier");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let job = fixture.json(&["__internal", "job", "--identifier", identifier]);
        if job["state"] == "succeeded" || job["state"] == "failed" {
            return job;
        }
        assert!(Instant::now() < deadline, "backup did not finish");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn publishes_private_immutable_generations_and_retains_two() {
    let fixture = prepared();
    for answer in [1, 2, 3] {
        fixture.write(
            "root/.claude/settings.json",
            &format!("{{\"answer\":{answer}}}"),
        );
        let job = backup(&fixture);
        assert_eq!(job["state"], "succeeded", "{job}");
    }
    let root = fixture.root().join("mnt/user/appdata/claude-code");
    assert_eq!(
        fs::read_dir(root.join("generations"))
            .expect("generations")
            .count(),
        2
    );
    let selected = root.join(fs::read_link(root.join("current")).expect("selector"));
    assert_eq!(
        fs::read_to_string(selected.join("root/.claude/settings.json")).expect("settings"),
        "{\"answer\":3}"
    );
    assert_eq!(
        fs::metadata(selected.join("root/.claude.json"))
            .expect("credential mode")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn failed_transfer_preserves_previous_selection_and_success_time() {
    let fixture = prepared();
    assert_eq!(backup(&fixture)["state"], "succeeded");
    let root = fixture.root().join("mnt/user/appdata/claude-code");
    let before = fs::read_link(root.join("current")).expect("selector");
    let receipt = fs::read(root.join("last-success.json")).expect("success receipt");
    fixture.executable("bin/rsync", "#!/bin/sh\nexit 23\n");
    assert_eq!(backup(&fixture)["state"], "failed");
    assert_eq!(
        fs::read_link(root.join("current")).expect("selector"),
        before
    );
    assert_eq!(
        fs::read(root.join("last-success.json")).expect("success receipt"),
        receipt
    );
}

#[test]
fn missing_mount_or_restore_marker_cannot_publish() {
    let fixture = prepared();
    fs::rename(
        fixture.root().join(".storage-mounted"),
        fixture.root().join(".storage-hidden"),
    )
    .expect("simulate unmount");
    assert_eq!(backup(&fixture)["state"], "failed");
    assert!(!fixture.root().join("mnt/user/appdata/claude-code").exists());
}
