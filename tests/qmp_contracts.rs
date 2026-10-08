use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    thread,
    time::{Duration, Instant},
};
use xdj_backend::{
    cli::Args,
    controls::{Controls, integer, key},
    qmp::{Qmp, bounded_line},
};
fn peer() -> (Qmp, thread::JoinHandle<Vec<Value>>) {
    let (client, server) = UnixStream::pair().unwrap();
    let writer = client.try_clone().unwrap();
    let handle = thread::spawn(move || {
        let mut input = BufReader::new(server.try_clone().unwrap());
        let mut output = server;
        writeln!(output, "{}", json!({"QMP":{}})).unwrap();
        let mut requests = Vec::new();
        for _ in 0..9 {
            let mut line = String::new();
            if input.read_line(&mut line).unwrap() == 0 {
                break;
            }
            let message: Value = serde_json::from_str(&line).unwrap();
            requests.push(message.clone());
            writeln!(output, "{}", json!({"event":"ASYNC"})).unwrap();
            writeln!(output, "{}", json!({"id":999,"return":null})).unwrap();
            let reply = if message["execute"] == "bad" {
                json!({"id":message["id"],"error":{"desc":"deliberate failure"}})
            } else {
                json!({"id":message["id"],"return":{}})
            };
            writeln!(output, "{reply}").unwrap();
        }
        requests
    });
    (Qmp::connect(client, writer).unwrap(), handle)
}
#[test]
fn control_bridge_ignores_events_releases_holds_and_validates_types() {
    let (mut qmp, handle) = peer();
    let mut controls = Controls::default();
    controls.button(&mut qmp, "play", true).unwrap();
    controls.button(&mut qmp, "play", true).unwrap();
    controls.touch(&mut qmp, 799, 479, true).unwrap();
    controls
        .apply(&mut qmp, &json!({"action":"pitch","value":65535}))
        .unwrap();
    controls.release(&mut qmp).unwrap();
    assert!(qmp.execute("bad", json!({})).is_err());
    assert!(
        controls
            .apply(&mut qmp, &json!({"action":"pitch","value":true}))
            .is_err()
    );
    drop(qmp);
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 7);
    assert_eq!(
        requests[2]["arguments"]["events"][0]["data"]["value"],
        32767
    );
    assert_eq!(requests[4]["arguments"]["events"][0]["data"]["down"], false);
}
#[test]
fn qmp_silence_is_bounded() {
    let (client, mut server) = UnixStream::pair().unwrap();
    let writer = client.try_clone().unwrap();
    let handle = thread::spawn(move || {
        writeln!(server, "{}", json!({"QMP":{}})).unwrap();
        let mut input = BufReader::new(server.try_clone().unwrap());
        let mut line = String::new();
        input.read_line(&mut line).unwrap();
        let id: Value = serde_json::from_str(&line).unwrap();
        writeln!(server, "{}", json!({"id":id["id"],"return":{}})).unwrap();
        line.clear();
        input.read_line(&mut line).unwrap();
        thread::sleep(Duration::from_millis(200));
    });
    let mut qmp = Qmp::connect(client, writer).unwrap();
    qmp.timeout = Duration::from_millis(30);
    let begin = Instant::now();
    assert!(qmp.execute("no-reply", json!({})).is_err());
    assert!(begin.elapsed() < Duration::from_millis(150));
    drop(qmp);
    handle.join().unwrap();
}
#[test]
fn lines_and_cli_arguments_are_bounded_and_strict() {
    assert!(bounded_line(&mut BufReader::new(&b"abcdef\n"[..]), 4).is_err());
    assert_eq!(
        bounded_line(&mut BufReader::new(&b"abc\nrest"[..]), 4).unwrap(),
        b"abc\n"
    );
    for value in [json!(true), json!(2.5), json!(-1), json!(65536)] {
        assert!(integer(&value, 0, 65535).is_err());
    }
    assert!(key("invented-button").is_err());
    assert!(Args::parse(["shell", "--unknown"].map(String::from)).is_err());
    assert!(Args::parse(["shell", "--firmware"].map(String::from)).is_err());
    assert!(Args::parse(["shell", "--mute-audio", "--unmute-audio"].map(String::from)).is_err());
}
