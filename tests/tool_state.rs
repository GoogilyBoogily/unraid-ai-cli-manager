#![cfg(feature = "test-support")]
mod common;
use common::Fixture;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    thread,
    time::{Duration, Instant},
};

fn prepared() -> Fixture {
    let fixture = Fixture::new();
    fixture.mount();
    let release = "root/.codex/packages/standalone/releases/1.0.0-x86_64-unknown-linux-musl";
    fixture.write(&format!("{release}/codex-package.json"), &json!({"layoutVersion":1,"version":"1.0.0","entrypoint":"bin/codex","execServer":"bin/codex-exec-server"}).to_string());
    for helper in [
        "bin/codex",
        "bin/codex-code-mode-host",
        "bin/codex-exec-server",
        "codex-path/rg",
        "codex-resources/bwrap",
    ] {
        fixture.executable(
            &format!("{release}/{helper}"),
            "#!/bin/sh\necho codex-cli 1.0.0\n",
        );
    }
    fs::create_dir_all(fixture.root().join("root/.local/bin")).expect("launcher parent");
    symlink(
        fixture.root().join(release),
        fixture
            .root()
            .join("root/.codex/packages/standalone/current"),
    )
    .expect("current package");
    symlink(
        fixture
            .root()
            .join("root/.codex/packages/standalone/current/bin/codex"),
        fixture.root().join("root/.local/bin/codex"),
    )
    .expect("launcher");
    fixture.write("root/.codex/auth.json", "{\"token\":\"fixture-secret\"}");
    fs::set_permissions(
        fixture.root().join("root/.codex/auth.json"),
        fs::Permissions::from_mode(0o777),
    )
    .expect("legacy permissions");
    fixture.write("root/.codex-persist-restored", "restored");
    fixture.executable("bin/rsync", "#!/bin/sh\nexec /usr/bin/rsync \"$@\"\n");
    fixture
}

fn complete(fixture: &Fixture, operation: &str) -> Value {
    let receipt = fixture.json(&[operation, "--tool", "codex", "--json"]);
    let identifier = receipt["jobs"][0]["receipt"]["job_identifier"]
        .as_str()
        .expect("identifier");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let job = fixture.json(&["__internal", "job", "--identifier", identifier]);
        if ["succeeded", "failed"].contains(&job["state"].as_str().expect("state")) {
            return job;
        }
        assert!(Instant::now() < deadline, "job didn't complete: {job}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn sqlite_snapshot_is_consistent_and_cold_restore_keeps_complete_package() {
    let fixture = prepared();
    let source = rusqlite::Connection::open(fixture.root().join("root/.codex/state.sqlite"))
        .expect("live database");
    source
        .execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE state(value); INSERT INTO state VALUES(7);",
        )
        .expect("WAL data");
    assert_eq!(complete(&fixture, "backup")["state"], "succeeded");
    let current = fixture
        .root()
        .join("mnt/user/appdata/codex/current/root/.codex");
    assert_eq!(
        fs::metadata(current.join("auth.json"))
            .expect("auth")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let database = rusqlite::Connection::open(current.join("state.sqlite")).expect("snapshot");
    assert_eq!(
        database
            .query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
            .expect("quick_check"),
        "ok"
    );
    drop(database);
    drop(source);
    fs::rename(
        fixture.root().join("root/.codex"),
        fixture.root().join("root/.codex-before-reboot"),
    )
    .expect("RAM loss");
    fs::rename(
        fixture.root().join("root/.codex-persist-restored"),
        fixture.root().join("root/.marker-before-reboot"),
    )
    .expect("marker loss");
    fs::rename(
        fixture.root().join("root/.local/bin/codex"),
        fixture.root().join("root/.local/bin/codex-before-reboot"),
    )
    .expect("launcher loss");
    assert_eq!(complete(&fixture, "restore")["state"], "succeeded");
    assert!(
        fixture
            .root()
            .join("root/.codex/packages/standalone/current/codex-path/rg")
            .is_file()
    );
    assert!(
        fixture
            .root()
            .join("root/.codex-persist-restored")
            .is_file()
    );
}

#[test]
fn missing_helper_and_corrupt_database_prevent_publication() {
    let fixture = prepared();
    fixture.write("root/.codex/broken.sqlite", "not a database");
    assert_eq!(complete(&fixture, "backup")["state"], "failed");
    assert!(
        !fixture
            .root()
            .join("mnt/user/appdata/codex/current")
            .exists()
    );
}
