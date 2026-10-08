mod common;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};
use xdj_backend::{
    cache,
    runtime::{Options, Runtime},
    state::State,
    util,
};
fn native(root: &Path) {
    let origin = Path::new(env!("CARGO_MANIFEST_DIR")).join("native/qemu/hw/cdj/c6x");
    let target = root.join("native/qemu/hw/cdj/c6x");
    fs::create_dir_all(target.join("tools")).unwrap();
    for name in [
        "c66x_priv.h",
        "c66x_core_int.h",
        "c66x.h",
        "c66x_decode.c",
        "tools/c14_jitgen.py",
    ] {
        fs::copy(origin.join(name), target.join(name)).unwrap();
    }
}
fn fixture(root: &Path) -> std::path::PathBuf {
    let engine = root.join("fake-qemu");
    fs::write(&engine, include_bytes!("fixtures/qmp-peer.sh")).unwrap();
    fs::set_permissions(&engine, fs::Permissions::from_mode(0o755)).unwrap();
    engine
}
#[test]
fn explicit_validation_does_not_reuse_a_previous_verdict() {
    let dir = tempfile::tempdir().unwrap();
    native(dir.path());
    let source = dir.path().join("input.UPD");
    fs::write(&source, common::package()).unwrap();
    let state = State::prepare(&source, &dir.path().join("state")).unwrap();
    let engine = fixture(dir.path());
    let identity = cache::identity(dir.path(), &engine, &state).unwrap();
    let cache_path = cache::prepare(&state, &identity).unwrap();
    let batch = cache_path.join("batch1");
    fs::create_dir(&batch).unwrap();
    fs::write(batch.join("m.so"), b"inventory fixture").unwrap();
    fs::write(
        batch.join("m.cache"),
        format!("{}\n", cache::MODULE_VERSION),
    )
    .unwrap();
    util::atomic_json(
        &cache_path.join("preparation.json"),
        &json!({"schema":1,"profile":3,"status":"unqualified","identity":identity,
            "modules":cache::inventory(&cache_path).unwrap()}),
    )
    .unwrap();
    assert!(cache::status(&state, &identity).is_some());
    let qemu_img = dir.path().join("qemu-img");
    fs::write(
        &qemu_img,
        b"#!/bin/sh\nfor last do :; done\n: > \"$last\"\n",
    )
    .unwrap();
    fs::set_permissions(&qemu_img, fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_xdj-backend"))
        .args(["shell", "--validate", "--audio", "off"])
        .arg("--mp3-image")
        .arg(dir.path().join("missing-mp3.img"))
        .arg("--root")
        .arg(dir.path())
        .arg("--state")
        .arg(&state.directory)
        .arg("--engine")
        .arg(engine)
        .arg("--firmware")
        .arg(source)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut input = child.stdin.take().unwrap();
    let mut validation = Value::Null;
    loop {
        let mut line = String::new();
        assert!(output.read_line(&mut line).unwrap() > 0);
        let event: Value = serde_json::from_str(&line).unwrap();
        if event["event"] == "prepared" {
            validation = event.clone();
        }
        if event["event"] == "running" {
            break;
        }
    }
    writeln!(input, "{}", json!({"action":"quit"})).unwrap();
    drop(input);
    let _ = output.lines().collect::<Result<Vec<_>, _>>().unwrap();
    assert!(child.wait().unwrap().success());
    assert!(
        validation["message"]
            .as_str()
            .unwrap()
            .starts_with("Validation failed:"),
        "Explicit validation must rerun, even with a compatible prior verdict: {validation}"
    );
}
#[test]
fn runtime_quits_only_its_child_and_releases_state() {
    let dir = tempfile::tempdir().unwrap();
    native(dir.path());
    let source = dir.path().join("input.UPD");
    fs::write(&source, common::package()).unwrap();
    let state = State::prepare(&source, &dir.path().join("state")).unwrap();
    let mut options = Options::at(dir.path());
    options.engine = fixture(dir.path());
    options.audio = "off".into();
    options.mode = "interpreter".into();
    let runtime = Runtime::start(&state, &options).unwrap();
    let pid = runtime.pid();
    assert!(xdj_backend::state::Lease::acquire(&state.directory).is_err());
    drop(runtime);
    // SAFETY: signal zero only checks existence of the captured child PID; it sends no signal.
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    assert!(xdj_backend::state::Lease::acquire(&state.directory).is_ok());
}
#[test]
fn changed_engine_retires_code_and_never_overwrites_nor() {
    let dir = tempfile::tempdir().unwrap();
    native(dir.path());
    let source = dir.path().join("input.UPD");
    fs::write(&source, common::package()).unwrap();
    let state = State::prepare(&source, &dir.path().join("state")).unwrap();
    let engine = fixture(dir.path());
    let first = cache::identity(dir.path(), &engine, &state).unwrap();
    let cache_path = cache::prepare(&state, &first).unwrap();
    fs::write(cache_path.join("code"), b"preserved").unwrap();
    let flash = util::file_hash(&state.directory.join("flash.bin")).unwrap();
    fs::write(&engine, b"changed engine").unwrap();
    let second = cache::identity(dir.path(), &engine, &state).unwrap();
    cache::prepare(&state, &second).unwrap();
    assert_eq!(
        util::file_hash(&state.directory.join("flash.bin")).unwrap(),
        flash
    );
    assert!(
        fs::read_dir(&state.directory)
            .unwrap()
            .filter_map(Result::ok)
            .any(|p| p
                .file_name()
                .to_string_lossy()
                .starts_with("dsp-jit-retired-"))
    );
}
#[test]
fn stdio_shell_recovers_from_bad_inputs_and_closes_on_quit() {
    let dir = tempfile::tempdir().unwrap();
    native(dir.path());
    let source = dir.path().join("input.UPD");
    fs::write(&source, common::package()).unwrap();
    let engine = fixture(dir.path());
    let image = dir.path().join("input.img");
    fs::write(&image, b"mock backing image").unwrap();
    let qemu_img = dir.path().join("qemu-img");
    fs::write(
        &qemu_img,
        b"#!/bin/sh\nfor last do :; done\n: > \"$last\"\n",
    )
    .unwrap();
    fs::set_permissions(&qemu_img, fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_xdj-backend"))
        .arg("shell")
        .arg("--root")
        .arg(dir.path())
        .arg("--engine")
        .arg(&engine)
        .arg("--firmware")
        .arg(&source)
        .args(["--audio", "off", "--compile-mode", "interpreter"])
        .arg("--usb-image")
        .arg(image)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut input = child.stdin.take().unwrap();
    let mut seen = Vec::new();
    loop {
        let mut line = String::new();
        assert!(output.read_line(&mut line).unwrap() > 0);
        let event: Value = serde_json::from_str(&line).unwrap();
        let running = event["event"] == "running";
        seen.push(event);
        if running {
            break;
        }
    }
    writeln!(
        input,
        "{}",
        json!({"action":"button","name":"play","down":true})
    )
    .unwrap();
    writeln!(input, "{}", json!({"action":"pitch","value":true})).unwrap();
    writeln!(
        input,
        "{}",
        json!({"action":"firmware","path":"/does/not/exist"})
    )
    .unwrap();
    writeln!(input, "{}", json!({"action":"quit"})).unwrap();
    drop(input);
    for line in output.lines() {
        seen.push(serde_json::from_str(&line.unwrap()).unwrap());
    }
    let status = child.wait().unwrap();
    assert!(status.success());
    assert_eq!(seen.last().unwrap()["event"], "stopped");
    assert_eq!(
        seen.iter()
            .filter(|e| e["event"] == "error" && e["recoverable"] == true)
            .count(),
        2
    );
    assert!(
        seen.iter()
            .any(|e| e["event"] == "running" && e["backend"] == "rust")
    );
}
#[test]
fn shell_media_switches_diagnostics_and_restarts_preserve_protocol() {
    let dir = tempfile::tempdir().unwrap();
    native(dir.path());
    let source = dir.path().join("a.UPD");
    let second = dir.path().join("b.UPD");
    fs::write(&source, common::package()).unwrap();
    fs::write(&second, common::variant("1.46")).unwrap();
    let engine = fixture(dir.path());
    let image = dir.path().join("input.img");
    fs::write(&image, b"backing").unwrap();
    let song = dir.path().join("song.mp3");
    fs::write(&song, b"audio").unwrap();
    let export = dir.path().join("USB");
    fs::create_dir_all(export.join("PIONEER/rekordbox")).unwrap();
    fs::write(export.join("PIONEER/rekordbox/export.pdb"), b"db").unwrap();
    let qemu_img = dir.path().join("qemu-img");
    fs::write(
        &qemu_img,
        b"#!/bin/sh\nfor last do :; done\n: > \"$last\"\n",
    )
    .unwrap();
    fs::set_permissions(&qemu_img, fs::Permissions::from_mode(0o755)).unwrap();
    let log = dir.path().join("qmp.log");
    let mut child = Command::new(env!("CARGO_BIN_EXE_xdj-backend"))
        .env("MOCK_QMP_LOG", &log)
        .arg("shell")
        .arg("--root")
        .arg(dir.path())
        .arg("--engine")
        .arg(engine)
        .arg("--firmware")
        .arg(&source)
        .args(["--audio", "off", "--compile-mode", "interpreter"])
        .arg("--usb-image")
        .arg(&image)
        .arg("--diagnostics")
        .arg(dir.path().join("diagnostics"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut input = child.stdin.take().unwrap();
    let mut events = Vec::<Value>::new();
    fn until_running(output: &mut impl BufRead, events: &mut Vec<Value>) {
        loop {
            let mut line = String::new();
            assert!(output.read_line(&mut line).unwrap() > 0);
            let event: Value = serde_json::from_str(&line).unwrap();
            let running = event["event"] == "running";
            assert_ne!(event["recoverable"], false);
            events.push(event);
            if running {
                break;
            }
        }
    }
    until_running(&mut output, &mut events);
    for message in [
        json!({"action":"touch","x":10,"y":20,"down":true}),
        json!({"action":"jog","ticks":32}),
        json!({"action":"vinyl-speed","value":255}),
        json!({"action":"mute","muted":true}),
        json!({"action":"release"}),
        json!({"action":"capture","name":"audio"}),
        json!({"action":"capture-stop"}),
        json!({"action":"screenshot"}),
        json!({"action":"firmware","path":source}),
        json!({"action":"restart"}),
    ] {
        writeln!(input, "{message}").unwrap();
    }
    input.flush().unwrap();
    until_running(&mut output, &mut events);
    for message in [
        json!({"action":"music","paths":[song]}),
        json!({"action":"export","folder":export}),
        json!({"action":"image","path":image}),
        json!({"action":"firmware","path":second}),
        json!({"action":"diagnostic"}),
    ] {
        writeln!(input, "{message}").unwrap();
        input.flush().unwrap();
        until_running(&mut output, &mut events);
    }
    writeln!(input, "{}", json!({"action":"stop"})).unwrap();
    drop(input);
    for line in output.lines() {
        events.push(serde_json::from_str(&line.unwrap()).unwrap());
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(events.iter().filter(|e| e["event"] == "running").count(), 7);
    assert!(events.iter().any(|e| e["event"] == "captured"));
    assert!(
        events
            .iter()
            .any(|e| e["event"] == "selected" && e["unchanged"] == true)
    );
    assert!(!events.iter().any(|e| e["event"] == "error"));
    let commands = fs::read_to_string(log).unwrap();
    assert!(commands.contains("xdj-panel-analog1"));
    assert!(commands.contains("stopcapture 0"));
}
#[test]
fn prepared_launch_requires_complete_batches_and_invalid_greeting_keeps_log() {
    let dir = tempfile::tempdir().unwrap();
    native(dir.path());
    let source = dir.path().join("input.UPD");
    fs::write(&source, common::package()).unwrap();
    let state = State::prepare(&source, &dir.path().join("state")).unwrap();
    let mut options = Options::at(dir.path());
    options.engine = fixture(dir.path());
    options.audio = "off".into();
    options.mode = "prepared".into();
    assert!(Runtime::start(&state, &options).is_err());
    let batch = state.directory.join("dsp-jit/batch1");
    fs::create_dir(&batch).unwrap();
    fs::write(batch.join("m.cache"), cache::MODULE_VERSION).unwrap();
    fs::write(batch.join("m.so"), b"fixture").unwrap();
    let runtime = Runtime::start(&state, &options).unwrap();
    drop(runtime);
    fs::write(
        &options.engine,
        b"#!/bin/sh\necho actual-engine-startup-error >&2\necho '{}'\n",
    )
    .unwrap();
    options.mode = "interpreter".into();
    let error = Runtime::start(&state, &options).err().unwrap().to_string();
    assert!(error.contains("actual-engine-startup-error"));
    assert!(xdj_backend::state::Lease::acquire(&state.directory).is_ok());
}
