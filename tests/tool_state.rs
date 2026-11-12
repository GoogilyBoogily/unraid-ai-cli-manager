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

#[test]
fn deleted_databases_disappear_and_runtime_links_are_excluded() {
    let fixture = prepared();
    let database = fixture.root().join("root/.codex/deleted.sqlite");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch("CREATE TABLE state(value); INSERT INTO state VALUES(1)")
        .unwrap();
    drop(connection);
    symlink(
        "/tmp/codex-runtime.sock",
        fixture.root().join("root/.codex/runtime-channel"),
    )
    .unwrap();
    let external = fixture.root().join("external-state");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("keep.txt"), "external-owned").unwrap();
    symlink(&external, fixture.root().join("root/.codex/external-link")).unwrap();
    assert_eq!(complete(&fixture, "backup")["state"], "succeeded");
    let snapshot = fixture
        .root()
        .join("mnt/user/appdata/codex/current/root/.codex");
    assert!(!snapshot.join("runtime-channel").is_symlink());
    assert_eq!(
        fs::read_link(snapshot.join("external-link")).unwrap(),
        external
    );
    assert!(snapshot.join("deleted.sqlite").is_file());
    fs::rename(
        database,
        fixture.root().join("deleted-before-second-backup.sqlite"),
    )
    .unwrap();
    assert_eq!(complete(&fixture, "backup")["state"], "succeeded");
    assert!(!snapshot.join("deleted.sqlite").exists());
}

#[test]
fn database_created_during_rsync_cannot_publish_a_main_file_without_its_wal() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
    };
    let fixture = prepared();
    assert_eq!(complete(&fixture, "backup")["state"], "succeeded");
    let selector = fixture.root().join("mnt/user/appdata/codex/current");
    let previous = fs::read_link(&selector).unwrap();
    let mut writer = Command::new("python3")
        .args([
            "-u",
            "-c",
            r#"
import os,pathlib,sqlite3,sys,time
root=pathlib.Path(sys.argv[1])
print('ready',flush=True)
while not (root/'create-database').exists(): time.sleep(.005)
connection=sqlite3.connect(root/'root/.codex/new.sqlite')
connection.execute('PRAGMA journal_mode=WAL')
connection.execute('CREATE TABLE fresh(value)')
connection.execute('INSERT INTO fresh VALUES(42)')
connection.commit()
(root/'database-created').write_text('ready')
sys.stdin.read()
"#,
            fixture.root().to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = String::new();
    BufReader::new(writer.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "ready");
    fixture.executable("bin/rsync","#!/bin/sh\ntouch \"$AI_CLI_MANAGER_TEST_ROOT/create-database\"\nwhile [ ! -e \"$AI_CLI_MANAGER_TEST_ROOT/database-created\" ]; do sleep 0.01; done\nexec /usr/bin/rsync \"$@\"\n");
    let job = complete(&fixture, "backup");
    writer.stdin.take().unwrap().write_all(b"").unwrap();
    assert!(writer.wait().unwrap().success());
    assert_eq!(job["state"], "failed", "{job}");
    assert_eq!(job["error_code"], "database_set_changed");
    assert_eq!(fs::read_link(selector).unwrap(), previous);
}

#[test]
fn relative_external_authentication_link_is_preserved_without_copying_its_target() {
    let fixture = prepared();
    fs::rename(
        fixture.root().join("root/.codex/auth.json"),
        fixture.root().join("root/auth-external.json"),
    )
    .unwrap();
    symlink(
        "../auth-external.json",
        fixture.root().join("root/.codex/auth.json"),
    )
    .unwrap();
    let job = complete(&fixture, "backup");
    assert_eq!(job["state"], "succeeded", "{job}");
    let root = fixture.root().join("mnt/user/appdata/codex/current/root");
    assert_eq!(
        fs::read_link(root.join(".codex/auth.json")).unwrap(),
        std::path::Path::new("../auth-external.json")
    );
    assert!(!root.join("auth-external.json").exists());
}

#[test]
fn absolute_external_authentication_link_is_preserved_and_missing_target_blocks_readiness() {
    let fixture = prepared();
    let external = fixture.root().join("root/auth-external.json");
    fs::rename(fixture.root().join("root/.codex/auth.json"), &external).unwrap();
    symlink(&external, fixture.root().join("root/.codex/auth.json")).unwrap();
    let job = complete(&fixture, "backup");
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(
        fs::read_link(
            fixture
                .root()
                .join("mnt/user/appdata/codex/current/root/.codex/auth.json")
        )
        .unwrap(),
        external
    );
    fs::rename(
        &external,
        fixture
            .root()
            .join("root/auth-external-before-removal.json"),
    )
    .unwrap();
    assert_eq!(
        complete(&fixture, "backup")["error_code"],
        "missing_external_target"
    );
}

#[test]
fn missing_or_escaping_package_helper_cannot_be_committed() {
    for escaping in [false, true] {
        let fixture = prepared();
        let helper=fixture.root().join("root/.codex/packages/standalone/releases/1.0.0-x86_64-unknown-linux-musl/codex-resources/bwrap");
        let external = fixture.root().join("external-helper");
        fs::rename(&helper, &external).unwrap();
        if escaping {
            symlink(external, &helper).unwrap();
        }
        let job = complete(&fixture, "backup");
        assert_eq!(job["state"], "failed", "{job}");
        assert!(
            !fixture
                .root()
                .join("mnt/user/appdata/codex/current")
                .exists()
        );
    }
}
