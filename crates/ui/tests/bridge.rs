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

#[test]
fn idle_ui_backpressures_events_and_can_resume_or_close_a_full_queue() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let backend = Path::new(env!("CARGO_BIN_EXE_xdj-ui-fixture"));
    let notifications = Arc::new(AtomicUsize::new(0));
    let wake = notifications.clone();
    let mut bridge = Bridge::spawn(
        backend,
        &[],
        dir.path(),
        Arc::new(move || {
            wake.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();
    // Leave more acknowledgements than fit in the UI queue unread, as an
    // occluded/minimized window can do while the player keeps reporting state.
    for seq in 0..400 {
        loop {
            if bridge
                .send(json!({"action":"bench-echo","seq":seq}))
                .is_ok()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while notifications.load(Ordering::Relaxed) < 256 {
        assert!(
            Instant::now() < deadline,
            "Fixture did not fill the event queue"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(30));
    assert!(
        bridge.error.lock().unwrap().is_none(),
        "Full event queues must not close the backend output reader"
    );
    assert!(!bridge.exited().unwrap());
    let mut acknowledgements = 0;
    while acknowledgements < 400 {
        let event = bridge.events.recv_timeout(Duration::from_secs(3)).unwrap();
        if event["event"] == "input-ack" {
            assert_eq!(event["command"]["seq"], acknowledgements);
            acknowledgements += 1;
        }
    }
    // Fill it again and close without draining: blocked readers must be joined.
    for seq in 0..400 {
        while bridge
            .send(json!({"action":"bench-echo","seq":seq}))
            .is_err()
        {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while notifications.load(Ordering::Relaxed) < 400 + 3 + 256 {
        assert!(
            Instant::now() < deadline,
            "Fixture did not refill the event queue"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let start = Instant::now();
    drop(bridge);
    assert!(start.elapsed() < Duration::from_secs(2));
}
