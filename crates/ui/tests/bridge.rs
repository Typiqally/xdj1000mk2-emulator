use serde_json::json;
use std::{
    io::Cursor,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use xdj_ui::bridge::{Bridge, read_line};
#[test]
fn bridge_rejects_unbounded_or_partial_records() {
    assert!(read_line(&mut Cursor::new(b"12345\n"), 5).is_err());
    assert!(read_line(&mut Cursor::new(b"123"), 5).is_err());
    assert!(read_line(&mut Cursor::new(b""), 5).unwrap().is_none());
    assert_eq!(
        read_line(&mut Cursor::new(b"123\n"), 5).unwrap().unwrap(),
        b"123\n"
    );
}
#[test]
fn owned_child_receives_input_and_exits_with_its_display_socket() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Path::new(env!("CARGO_BIN_EXE_xdj-ui-fixture"));
    let bridge = Bridge::spawn(
        backend,
        &["--diagnostics".into(), dir.path().display().to_string()],
        dir.path(),
        Arc::new(|| {}),
    )
    .unwrap();
    let selected = bridge.events.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(selected["event"], "selected");
    let running = bridge.events.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(running["event"], "running");
    bridge
        .send(json!({"action":"button","name":"cue","down":true}))
        .unwrap();
    let start = Instant::now();
    loop {
        let event = bridge.events.recv_timeout(Duration::from_secs(3)).unwrap();
        if event["event"] == "input-ack" {
            assert_eq!(event["command"]["name"], "cue");
            assert!(event["peer_received_unix_us"].as_u64().is_some());
            break;
        }
    }
    drop(bridge);
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(
        !Path::new(
            running["vnc"]
                .as_str()
                .unwrap()
                .strip_prefix("unix:")
                .unwrap_or(running["vnc"].as_str().unwrap())
        )
        .exists()
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("fixture.json")).unwrap()).unwrap();
    assert!(
        report["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["command"]["action"] == "quit")
    );
}

#[test]
fn benchmark_peer_really_streams_pixels_after_its_json_handshake() {
    use std::sync::Mutex;
    use xdj_ui::transport::{Display, Latest};
    let dir = tempfile::tempdir().unwrap();
    let backend = Path::new(env!("CARGO_BIN_EXE_xdj-ui-fixture"));
    let bridge = Bridge::spawn(backend, &[], dir.path(), Arc::new(|| {})).unwrap();
    assert_eq!(
        bridge.events.recv_timeout(Duration::from_secs(3)).unwrap()["event"],
        "selected"
    );
    let running = bridge.events.recv_timeout(Duration::from_secs(3)).unwrap();
    let latest = Arc::new(Latest::default());
    let error = Arc::new(Mutex::new(None));
    let display = Display::start(
        running["vnc"].as_str().unwrap(),
        latest.clone(),
        error.clone(),
        Arc::new(|| {}),
    )
    .unwrap();
    let start = Instant::now();
    loop {
        if let Some(frame) = latest.take() {
            assert_eq!(frame.size, [800, 480]);
            break;
        }
        assert!(
            error.lock().unwrap().is_none(),
            "{:?}",
            error.lock().unwrap()
        );
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(display);
    drop(bridge);
}
