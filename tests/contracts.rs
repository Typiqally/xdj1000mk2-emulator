use serde_json::json;
use xdj_backend::{firmware::lzss_decode, lamps, state::Lease};

#[test]
fn lzss_literals_and_overlapping_references() {
    assert_eq!(
        lzss_decode(&[7, b'A', b'B', b'C', 0xee, 0xf3]).unwrap(),
        b"ABCABCABC"
    );
}
#[test]
fn malformed_lzss_is_rejected() {
    assert!(lzss_decode(&[0]).is_err());
    assert!(lzss_decode(&[0, 1]).is_err());
}
#[test]
fn selected_engine_keeps_its_matching_data_and_explicit_paths_override_it() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("output");
    std::fs::create_dir(&output).unwrap();
    let engine = output.join("pinned-engine");
    let data = output.join("pinned-data");
    let alternate = output.join("alternate-engine");
    std::fs::write(&engine, b"engine").unwrap();
    std::fs::write(&alternate, b"alternate").unwrap();
    std::fs::create_dir(&data).unwrap();
    xdj_backend::util::atomic_json(
        &output.join("engine.json"),
        &json!({"engine":engine,"qemu_data":data}),
    )
    .unwrap();
    let args = xdj_backend::cli::Args::default();
    let options = xdj_backend::cli::options(&args, dir.path()).unwrap();
    assert_eq!(options.engine, engine.canonicalize().unwrap());
    assert_eq!(options.data, data.canonicalize().unwrap());
    let args = xdj_backend::cli::Args::parse([
        "shell".into(),
        "--engine".into(),
        alternate.to_string_lossy().into_owned(),
    ])
    .unwrap();
    let options = xdj_backend::cli::options(&args, dir.path()).unwrap();
    assert_eq!(options.engine, alternate.canonicalize().unwrap());
    assert_eq!(options.data, data.canonicalize().unwrap());
    std::fs::remove_file(engine).unwrap();
    assert!(xdj_backend::cli::options(&xdj_backend::cli::Args::default(), dir.path()).is_err());
}
#[test]
fn state_has_only_one_writer_and_can_be_released() {
    let dir = tempfile::tempdir().unwrap();
    let owner = Lease::acquire(dir.path()).unwrap();
    assert!(Lease::acquire(dir.path()).is_err());
    drop(owner);
    assert!(Lease::acquire(dir.path()).is_ok());
}
#[test]
fn linked_lock_cannot_lock_another_players_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("victim");
    std::fs::write(&target, b"unchanged").unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join(".run.lock")).unwrap();
    assert!(Lease::acquire(dir.path()).is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"unchanged");
}
#[test]
fn invalid_lamp_frame_leaves_lamps_dark() {
    assert_eq!(
        lamps::decode("{}"),
        json!({"lit":[],"usb":[0,0,0],"jog":{"position":null,"vinyl":false,"touch":false}})
    );
}
#[test]
fn lamp_frame_checks_checksum_and_ignores_stack_residue() {
    let mut frame = [0u8; 32];
    frame[0] = 0x89; // play plus two residue bits
    frame[1] = 0xd1; // selector plus residue
    frame[8] = 0x28;
    frame[9] = 135;
    frame[22] = 40;
    let sum: u16 = frame[..30].iter().map(|v| u16::from(*v)).sum();
    frame[30] = ((sum & 255) + (sum >> 8)) as u8;
    frame[31] = 0x8f;
    let encoded: String = frame.iter().map(|b| format!("{b:02x}")).collect();
    let result = lamps::decode(&json!({"frame":encoded}).to_string());
    assert_eq!(result["lit"], json!(["play", "selector", "usb"]));
    assert_eq!(result["usb"], json!([255, 0, 0]));
    assert_eq!(
        result["jog"],
        json!({"position":135,"vinyl":true,"touch":true})
    );
}
