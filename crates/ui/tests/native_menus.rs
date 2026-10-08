#![cfg(target_os = "macos")]
//! Run with `cargo test -p xdj-ui --test native_menus -- --ignored` on a macOS desktop.
use serde_json::{Value, json};
use std::{fs, process::Command};

#[test]
#[ignore = "opens an owned macOS window with a synthetic backend; requires a desktop session"]
fn installed_appkit_menus_dispatch_archive_views_audio_and_recent_history() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("firmware/v145");
    fs::create_dir_all(&archive).unwrap();
    let firmware = archive.join("XDJ1KMK2.UPD");
    // The fixture peer does not emulate firmware, DSP or audio.
    fs::write(&firmware, b"synthetic input").unwrap();
    fs::write(
        root.path().join("firmware/manifest.json"),
        json!({"versions":[{"version":"1.45","upd_sha256":"synthetic"}]}).to_string(),
    )
    .unwrap();
    let diagnostics = root.path().join("diagnostics");
    let history = root.path().join("private-history");
    let script = root.path().join("script.json");
    fs::write(&script, json!([
        {"after":1.5,"action":"native-menu","path":["File","Archived Firmware","Firmware 1.45"]},
        {"after":4.0,"action":"native-menu","path":["View","Player View"]},
        {"after":4.5,"action":"native-menu","path":["Player","Mute Audio"]},
        {"after":5.0,"action":"native-menu","path":["View","Show Keys"]},
        {"after":5.5,"action":"native-menu","path":["Player","Show Live Logs"]},
        {"after":6.5,"action":"native-menu","path":["File","Recent Firmware","Clear Recent Firmware"]},
        {"after":7.5,"action":"quit"}
    ]).to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xdj-ui"))
        .arg("--root")
        .arg(root.path())
        .args(["--backend", env!("CARGO_BIN_EXE_xdj-ui-fixture")])
        .arg("--diagnostics")
        .arg(&diagnostics)
        .arg("--test-script")
        .arg(script)
        .args(["--exit-after", "18", "--", "--state-root"])
        .arg(&history)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // State assertions use the exit report, independent of asynchronous GPU screenshot delivery.
    let final_state: Value =
        serde_json::from_slice(&fs::read(diagnostics.join("metrics.json")).unwrap()).unwrap();
    assert_eq!(
        final_state["native_menu"]
            .as_array()
            .unwrap()
            .iter()
            .skip(1)
            .map(|item| item["label"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["File", "Player", "View"]
    );
    assert_eq!(
        final_state["native_menu"][1]["children"][2]["children"][0]["label"],
        "Firmware 1.45"
    );
    assert_eq!(final_state["ready"], true);
    assert_eq!(final_state["has_frame"], true);
    assert_eq!(final_state["view"], "player");
    assert_eq!(final_state["muted"], false);
    assert!(
        final_state["recent_firmware"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let events = fs::read_to_string(diagnostics.join("app-events.jsonl")).unwrap();
    assert!(
        events
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|event| event["event"] == "selected"
                && event["requested_source"] == firmware.canonicalize().unwrap().to_str().unwrap())
    );
    assert_eq!(fs::read(firmware).unwrap(), b"synthetic input");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(history.join("recent-firmware.json")).unwrap())
            .unwrap(),
        json!([])
    );
}
