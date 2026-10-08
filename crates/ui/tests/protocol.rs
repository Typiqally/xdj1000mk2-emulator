use serde_json::json;
use std::io::{Cursor, Read, Write};
use xdj_ui::{
    app::Options,
    input::{Input, lcd_point},
    model::Player,
    rfb::{Framebuffer, handshake, update},
};
struct Peer {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
}
impl Read for Peer {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.input.read(out)
    }
}
impl Write for Peer {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.output.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn peer(data: Vec<u8>) -> Peer {
    Peer {
        input: Cursor::new(data),
        output: Vec::new(),
    }
}
fn greeting() -> Vec<u8> {
    let mut data = b"RFB 003.008\n".to_vec();
    data.extend_from_slice(&[1, 1, 0, 0, 0, 0]);
    let mut init = [0; 24];
    init[1] = 2;
    init[3] = 1;
    data.extend_from_slice(&init);
    data
}
#[test]
fn protocol_negotiates_only_raw_shared_display_and_bounds_metadata() {
    let mut p = peer(greeting());
    let fb = handshake(&mut p).unwrap();
    assert_eq!((fb.width, fb.height), (2, 1));
    assert_eq!(&p.output[..12], b"RFB 003.008\n");
    assert_eq!(
        &p.output[p.output.len() - 10..],
        &[3, 0, 0, 0, 0, 0, 0, 2, 0, 1]
    );
    for index in [0, 12, 13, 14, 18, 20] {
        let mut data = greeting();
        data[index] = 255;
        assert!(handshake(&mut peer(data)).is_err(), "index {index}");
    }
    let mut data = greeting();
    data[38..42].copy_from_slice(&65537u32.to_be_bytes());
    assert!(handshake(&mut peer(data)).is_err());
    assert!(handshake(&mut peer(vec![])).is_err());
}
#[test]
fn raw_updates_resize_and_reject_unsupported_or_truncated_data() {
    let mut fb = Framebuffer::new(2, 1).unwrap();
    let mut raw = vec![0, 0, 0, 1, 0, 0, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0];
    raw.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
    let mut p = peer(raw.clone());
    assert!(update(&mut p, &mut fb).unwrap());
    assert_eq!(fb.rgba(), [255, 0, 0, 255, 0, 0, 255, 255]);
    assert!(!update(&mut peer(vec![0, 0, 0, 0]), &mut fb).unwrap());
    assert!(!update(&mut peer(vec![2]), &mut fb).unwrap());
    assert!(!update(&mut peer(vec![3, 0, 0, 0, 0, 0, 0, 1, 42]), &mut fb).unwrap());
    assert!(update(&mut peer(vec![3, 0, 0, 0, 0, 16, 0, 1]), &mut fb).is_err());
    assert!(update(&mut peer(vec![3, 0, 0, 0, 0, 0, 0, 1]), &mut fb).is_err());
    assert!(update(&mut peer(vec![4]), &mut fb).is_err());
    assert!(update(&mut peer(vec![0, 0, 16, 1]), &mut fb).is_err());
    raw[15] = 1;
    assert!(update(&mut peer(raw.clone()), &mut fb).is_err());
    raw[15] = 0;
    raw.pop();
    assert!(update(&mut peer(raw), &mut fb).is_err());
    let mut resize = vec![0, 0, 0, 1, 0, 0, 0, 0, 0, 3, 0, 2];
    resize.extend_from_slice(&(-223i32).to_be_bytes());
    assert!(update(&mut peer(resize), &mut fb).unwrap());
    assert_eq!((fb.width, fb.height), (3, 2));
}
#[test]
fn held_touch_and_jog_never_repeat_or_exceed_hardware_limits() {
    let mut input = Input::default();
    assert!(input.touch(-1, 0, true).is_none());
    assert!(input.end_touch().is_none());
    assert!(input.touch(1, 2, true).is_some());
    assert!(input.touch(1, 2, true).is_none());
    assert_eq!(input.end_touch().unwrap()["down"], false);
    assert!(input.jog_move([1., 0.]).is_none());
    assert!(input.jog_begin([1., 0.], 10.).is_some());
    assert_eq!(input.jog_move([0., 1.]).unwrap()["ticks"], 90);
    assert_eq!(input.jog_move([-1., 0.]).unwrap()["ticks"], 90);
    assert!(input.jog_end().is_some());
    assert!(input.jog_end().is_none());
    assert!(input.jog_begin([10., 0.], 10.).is_none());
    assert_eq!(input.jog_move([0., -10.]).unwrap()["ticks"], -90);
    assert!(input.release().is_none());
    assert!(lcd_point([0., 0., 1., 1.], [f32::NAN, 0.]).is_none());
}
#[test]
fn recoverability_catalog_lamps_and_logs_preserve_user_visible_state() {
    let mut player = Player::default();
    player.event(&json!({"event":"running","vnc":"fixture","source":"a","state":"s","firmware":{"version":"1.45"}}));
    player.event(&json!({"event":"selected","unchanged":true}));
    assert!(player.ready);
    player
        .event(&json!({"event":"lamps","lit":["play",null],"usb":[3,300,4],"jog":{"position":1}}));
    assert!(player.lit.contains("play"));
    assert_eq!(player.usb, [3, 255, 4]);
    player.event(&json!({"event":"audio","dropped_frames":4}));
    assert_eq!(player.audio["dropped_frames"], 4);
    player.event(&json!({"event":"catalog","firmwares":[{"path":"a"}]}));
    assert_eq!(player.catalog.len(), 1);
    player.event(&json!({"event":"error","recoverable":false,"message":"Failed"}));
    assert!(!player.ready);
    assert!(player.endpoint.is_none());
    player.event(&json!({"event":"selected","unchanged":false}));
    for _ in 0..300 {
        player.log("x".repeat(9000));
    }
    assert_eq!(player.logs.len(), 256);
    assert_eq!(player.logs[0].len(), 4096);
}
#[test]
fn options_keep_backend_paths_separate_and_reject_invalid_scripts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().display().to_string();
    let parse = |args: Vec<&str>| Options::parse(args.into_iter().map(str::to_owned));
    let options = parse(vec![
        "--root",
        &root,
        "--backend",
        "backend",
        "--firmware",
        "image.upd",
        "--view",
        "player",
        "--ui-scale",
        "1.25",
        "--exit-after",
        "3",
        "--",
        "--audio",
        "off",
    ])
    .unwrap();
    assert_eq!(options.view, "player");
    assert_eq!(options.backend_args, ["--audio", "off"]);
    assert_eq!(options.backend, std::path::Path::new("backend"));
    for args in [
        vec!["--view", "invalid"],
        vec!["--ui-scale", "NaN"],
        vec!["--exit-after", "-1"],
        vec!["--root"],
        vec!["--audio", "off"],
        vec!["--help"],
    ] {
        assert!(parse(args).is_err());
    }
    let script = dir.path().join("script.json");
    std::fs::write(
        &script,
        r#"[{"after":2,"action":"quit"},{"after":1,"action":"snapshot"}]"#,
    )
    .unwrap();
    let options = parse(vec!["--test-script", script.to_str().unwrap()]).unwrap();
    assert_eq!(options.script[0]["after"], 1);
    std::fs::write(&script, r#"[{"after":-1}]"#).unwrap();
    assert!(parse(vec!["--test-script", script.to_str().unwrap()]).is_err());
}
