use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use xdj_ui::transport::{Display, Latest};

#[test]
fn latest_mailbox_replaces_stale_frames() {
    let latest = Latest::default();
    latest.offer(1);
    latest.offer(2);
    assert_eq!(latest.take(), Some(2));
    assert_eq!(latest.take(), None);
}

#[test]
fn fragmented_tcp_display_delivers_pixels_and_cancels_blocked_read() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let peer = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        for c in b"RFB 003.008\n" {
            s.write_all(&[*c]).unwrap();
        }
        let mut version = [0; 12];
        s.read_exact(&mut version).unwrap();
        s.write_all(&[1, 1]).unwrap();
        let mut one = [0];
        s.read_exact(&mut one).unwrap();
        s.write_all(&[0; 4]).unwrap();
        s.read_exact(&mut one).unwrap();
        let mut init = [0; 24];
        init[1] = 2;
        init[3] = 1;
        s.write_all(&init).unwrap();
        let mut config = [0; 42];
        s.read_exact(&mut config).unwrap();
        s.write_all(&[
            0, 0, 0, 1, 0, 0, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0, 0, 0, 255, 0, 255, 0, 0, 0,
        ])
        .unwrap();
        let mut request = [0; 10];
        s.read_exact(&mut request).unwrap();
        assert!(s.read(&mut one).unwrap() == 0);
    });
    let latest = Arc::new(Latest::default());
    let error = Arc::new(Mutex::new(None));
    let display = Display::start(
        &format!("tcp:{address}"),
        latest.clone(),
        error.clone(),
        Arc::new(|| {}),
    )
    .unwrap();
    let start = Instant::now();
    let frame = loop {
        if let Some(frame) = latest.take() {
            break frame;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(frame.size, [2, 1]);
    assert_eq!(frame.rgba, vec![255, 0, 0, 255, 0, 0, 255, 255]);
    let stop = Instant::now();
    drop(display);
    assert!(stop.elapsed() < Duration::from_secs(1));
    peer.join().unwrap();
    assert!(error.lock().unwrap().is_none());
}

#[test]
fn display_errors_are_actionable_and_do_not_connect_remote_hosts() {
    let latest = Arc::new(Latest::default());
    let error = Arc::new(Mutex::new(None));
    assert!(
        Display::start(
            "tcp:192.0.2.1:5900",
            latest.clone(),
            error.clone(),
            Arc::new(|| {})
        )
        .is_err()
    );
    assert!(Display::start("tcp:not-a-socket", latest, error, Arc::new(|| {})).is_err());
}
