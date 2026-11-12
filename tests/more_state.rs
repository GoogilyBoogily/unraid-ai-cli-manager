#![cfg(feature = "test-support")]
mod common;
use std::{
    fs,
    os::unix::fs::symlink,
    thread,
    time::{Duration, Instant},
};

#[test]
fn dangling_required_configuration_link_blocks_backup() {
    let fixture = common::Fixture::new();
    fixture.mount();
    fixture.write("root/.claude-persist-restored", "restored");
    fs::create_dir_all(fixture.root().join("root/.claude")).expect("live state");
    fixture.executable(
        "root/.local/share/claude/versions/1.0.0",
        "#!/bin/sh\necho 1.0.0\n",
    );
    fs::create_dir_all(fixture.root().join("root/.local/bin")).expect("launcher directory");
    symlink(
        "../share/claude/versions/1.0.0",
        fixture.root().join("root/.local/bin/claude"),
    )
    .expect("launcher");
    symlink(
        "/tmp/missing-manager-fixture-config",
        fixture.root().join("root/.claude/settings.json"),
    )
    .expect("external config link");
    fixture.executable("bin/rsync", "#!/bin/sh\nexec /usr/bin/rsync \"$@\"\n");
    let receipt = fixture.json(&["backup", "--tool", "claude"]);
    let identifier = receipt["jobs"][0]["receipt"]["job_identifier"]
        .as_str()
        .expect("job");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let job = fixture.json(&["__internal", "job", "--identifier", identifier]);
        if job["state"] == "failed" {
            assert_eq!(job["error_code"], "missing_external_target");
            break;
        }
        assert_ne!(
            job["state"], "succeeded",
            "backup accepted a missing required target"
        );
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn missing_marker_source_package_or_dependency_cannot_publish() {
    for missing in ["marker", "source", "package", "dependency"] {
        let fixture = common::Fixture::new();
        fixture.mount();
        fixture.manager_programs();
        fixture.claude("1.0.0");
        let path = match missing {
            "marker" => "root/.claude-persist-restored",
            "source" => "root/.claude",
            "package" => "root/.local/share/claude/versions/1.0.0",
            _ => "bin/rsync",
        };
        fs::rename(
            fixture.root().join(path),
            fixture.root().join("held-out-fixture-entry"),
        )
        .unwrap();
        let job = fixture.wait(&fixture.json(&["backup", "--tool", "claude"]));
        assert_eq!(job["state"], "failed", "{missing}: {job}");
        assert!(
            !fixture
                .root()
                .join("mnt/user/appdata/claude-code/current")
                .exists()
        );
    }
}
