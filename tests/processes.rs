#![cfg(feature = "test-support")]
mod common;
use std::{
    fs, thread,
    time::{Duration, Instant},
};

#[test]
fn timeout_reaps_detached_writers_before_return() {
    let fixture = common::Fixture::new();
    fixture.executable("bin/probe", "#!/bin/sh\nsetsid sh -c 'trap \"\" TERM; while :; do echo tick >> \"$AI_CLI_MANAGER_TEST_ROOT/writes\"; sleep 0.01; done' &\nwait\n");
    let response = fixture.json(&[
        "__internal",
        "probe",
        "--tool",
        "claude",
        "--milliseconds",
        "150",
    ]);
    let identifier = response["job_identifier"].as_str().expect("job identifier");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let job = fixture.json(&["__internal", "job", "--identifier", identifier]);
        if job["state"] == "failed" {
            assert_eq!(job["error_code"], "deadline_exceeded");
            break;
        }
        assert!(Instant::now() < deadline, "job did not terminate: {job}");
        thread::sleep(Duration::from_millis(20));
    }
    let before = fs::metadata(fixture.root().join("writes"))
        .expect("writer ran")
        .len();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        fs::metadata(fixture.root().join("writes"))
            .expect("writes")
            .len(),
        before
    );
}

#[test]
fn another_job_for_the_same_tool_is_busy() {
    let fixture = common::Fixture::new();
    fixture.executable("bin/probe", "#!/bin/sh\nsleep 0.3\n");
    fixture.json(&[
        "__internal",
        "probe",
        "--tool",
        "codex",
        "--milliseconds",
        "500",
    ]);
    let output = fixture.command(&[
        "__internal",
        "probe",
        "--tool",
        "codex",
        "--milliseconds",
        "500",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("busy"));
    thread::sleep(Duration::from_millis(600));
}
