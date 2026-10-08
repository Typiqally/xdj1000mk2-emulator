mod common;
use serde_json::Value;
use std::{fs, process::Command};
fn command(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xdj-backend"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn standalone_commands_validate_import_and_create_media() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("test.UPD");
    fs::write(&input, common::package()).unwrap();
    let inspect = command(&["inspect", "--firmware", input.to_str().unwrap()]);
    assert!(inspect.status.success());
    let report: Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(report["version"], "1.45");
    let library = dir.path().join("library");
    let import = command(&[
        "prepare",
        "--firmware",
        input.to_str().unwrap(),
        "--state-root",
        library.to_str().unwrap(),
    ]);
    assert!(import.status.success());
    let tone = dir.path().join("tone.img");
    assert!(
        command(&["tone", "--out", tone.to_str().unwrap(), "--seconds", "1"])
            .status
            .success()
    );
    assert!(
        !command(&["tone", "--out", tone.to_str().unwrap(), "--seconds", "0"])
            .status
            .success()
    );
    assert!(command(&["hardware", "--check-header"]).status.success());
    assert!(command(&["help"]).status.success());
    assert!(!command(&["unknown"]).status.success());
}
#[test]
fn missing_media_cannot_publish_a_ready_validation() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.UPD");
    fs::write(&file, common::package()).unwrap();
    let result = command(&[
        "validate",
        "--firmware",
        file.to_str().unwrap(),
        "--state-root",
        dir.path().join("library").to_str().unwrap(),
    ]);
    assert!(!result.status.success());
}
#[test]
fn command_line_media_export_and_hardware_generation_are_independent() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("song.wav");
    fs::write(&song, b"audio").unwrap();
    let music = dir.path().join("music.img");
    assert!(
        command(&[
            "music",
            "--out",
            music.to_str().unwrap(),
            song.to_str().unwrap()
        ])
        .status
        .success()
    );
    let usb = dir.path().join("USB");
    fs::create_dir_all(usb.join("rekordbox")).unwrap();
    fs::write(usb.join("rekordbox/export.pdb"), b"db").unwrap();
    let contents = dir.path().join("audio/Contents");
    fs::create_dir_all(&contents).unwrap();
    fs::write(contents.join("song.wav"), b"music").unwrap();
    let exported = dir.path().join("export.img");
    assert!(
        command(&[
            "export",
            "--out",
            exported.to_str().unwrap(),
            "--audio-root",
            contents.to_str().unwrap(),
            usb.to_str().unwrap()
        ])
        .status
        .success()
    );
    fs::create_dir_all(dir.path().join("native/qemu/hw/cdj/boards/xdj1000mk2")).unwrap();
    fs::write(
        dir.path().join("native/hardware_profile.json"),
        include_bytes!("../native/hardware_profile.json"),
    )
    .unwrap();
    assert!(
        command(&[
            "hardware",
            "--root",
            dir.path().to_str().unwrap(),
            "--write-header"
        ])
        .status
        .success()
    );
    assert!(
        command(&[
            "hardware",
            "--root",
            dir.path().to_str().unwrap(),
            "--check-header"
        ])
        .status
        .success()
    );
}
