use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn validate(request: Value, authenticated: bool, method: &str, token: &str) -> Value {
    let script = format!(
        "require '{}'; $i=json_decode(stream_get_contents(STDIN),true,32,JSON_THROW_ON_ERROR);try {{ echo json_encode(ai_manager_request($i['request'],$i['authenticated'],$i['method'],$i['token'])); }}catch(Throwable $e){{echo json_encode(['rejected'=>true]);}}",
        concat!(env!("CARGO_MANIFEST_DIR"), "/unraid/include/request.php")
    );
    let mut child =
        Command::new(std::env::var("AI_CLI_MANAGER_PHP").unwrap_or_else(|_| "php".into()))
            .args(["-n", "-r", &script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("PHP CLI is required for web validation tests");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            json!({"request":request,"authenticated":authenticated,"method":method,"token":token})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn web_boundary_rejects_unauthenticated_csrf_and_command_injection() {
    let valid = json!({"action":"update","tool":"claude","csrf_token":"expected"});
    assert_eq!(
        validate(valid.clone(), true, "POST", "expected")["arguments"],
        json!(["update", "--tool", "claude", "--json"])
    );
    for (request, authenticated, method, token) in [
        (valid.clone(), false, "POST", "expected"),
        (valid.clone(), true, "GET", "expected"),
        (valid.clone(), true, "POST", "wrong"),
        (
            json!({"action":"update","tool":"claude;touch /tmp/injected","csrf_token":"expected"}),
            true,
            "POST",
            "expected",
        ),
        (
            json!({"action":"job","identifier":"../../secret","csrf_token":"expected"}),
            true,
            "POST",
            "expected",
        ),
        (
            json!({"action":"settings","tool":"claude","enabled":"true","csrf_token":"expected"}),
            true,
            "POST",
            "expected",
        ),
        (
            json!({"action":"initialize","tool":"codex","host_execution_opt_in":"yes","csrf_token":"expected"}),
            true,
            "POST",
            "expected",
        ),
        (
            json!({"action":"settings","tool":"codex","initialized":true,"csrf_token":"expected"}),
            true,
            "POST",
            "expected",
        ),
    ] {
        assert_eq!(
            validate(request, authenticated, method, token)["rejected"],
            true
        );
    }
}
#[test]
fn web_settings_and_explicit_consent_use_fixed_arguments_and_stdin() {
    let result = validate(
        json!({"action":"settings","tool":"claude","enabled":false,"automatic_updates":true,"backup_interval_minutes":30,"csrf_token":"expected"}),
        true,
        "POST",
        "expected",
    );
    assert_eq!(result["arguments"], json!(["__internal", "settings"]));
    let settings: Value = serde_json::from_str(result["stdin"].as_str().unwrap()).unwrap();
    assert_eq!(settings["enabled"], false);
    assert!(settings.get("csrf_token").is_none());
    let result = validate(
        json!({"action":"initialize","tool":"codex","host_execution_opt_in":true,"csrf_token":"expected"}),
        true,
        "POST",
        "expected",
    );
    assert_eq!(
        result["arguments"],
        json!([
            "__internal",
            "initialize",
            "--tool",
            "codex",
            "--host-execution-opt-in"
        ])
    );
}
