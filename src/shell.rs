use crate::{
    cache,
    cli::{self, Args},
    controls::Controls,
    lamps, media,
    qmp::bounded_line,
    runtime::Runtime,
    state::{self, Lease, State},
    util::{self, atomic_json, read_json},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
pub fn emit(event: &str, data: Value) -> Result<()> {
    let mut object = data.as_object().cloned().unwrap_or_default();
    object.insert("event".into(), json!(event));
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &object)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}
pub fn commands() -> Receiver<Value> {
    let (send, receive) = mpsc::sync_channel(256);
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        loop {
            match bounded_line(&mut input, 65536) {
                Ok(line) if line.is_empty() => break,
                Ok(line) => {
                    let message = serde_json::from_slice::<Value>(&line)
                        .ok()
                        .filter(Value::is_object)
                        .unwrap_or_else(
                            || json!({"action":"invalid","reason":"Command must be a JSON object"}),
                        );
                    if send.send(message).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let _ = send.send(json!({"action":"invalid","reason":e.to_string()}));
                    break;
                }
            }
        }
        let _ = send.send(json!({"action":"quit"}));
    });
    receive
}
fn path(message: &Value, key: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(
        message[key]
            .as_str()
            .with_context(|| format!("Missing {key}"))?,
    ))
}
fn memory_capture(message: &Value, directory: &Path) -> Result<(u64, u64, PathBuf)> {
    let address = message["address"]
        .as_u64()
        .context("Missing numeric address")?;
    let size = message["size"].as_u64().context("Missing numeric size")?;
    ensure!(
        (1..=1024 * 1024).contains(&size),
        "Memory capture must be 1 byte to 1 MiB"
    );
    ensure!(
        address <= u32::MAX.into() && address + size <= 1_u64 << 32,
        "Capture outside guest address space"
    );
    let name = message["name"].as_str().context("Missing capture name")?;
    ensure!(
        !name.is_empty()
            && name.len() <= 80
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "Invalid capture name"
    );
    let destination = directory.canonicalize()?.join(format!("{name}.bin"));
    util::regular(&destination)?;
    ensure!(!destination.exists(), "Preserve previous memory capture");
    Ok((address, size, destination))
}
fn selected(state: &State, requested: Option<&Path>, unchanged: bool) -> Result<()> {
    emit(
        "selected",
        json!({"firmware":state.info,"source":state.source,"state":state.directory,"requested_source":requested,"unchanged":unchanged}),
    )
}
pub fn run(args: &Args, root: &Path) -> Result<()> {
    let mut options = cli::options(args, root)?;
    let receive = commands();
    let mut pending = std::collections::VecDeque::new();
    emit("starting", json!({"message":"Starting firmware…"}))?;
    ensure!(
        options.engine.is_file(),
        "Emulator engine missing; run xdj-backend build-engine"
    );
    let library = args
        .path("state-root")
        .unwrap_or_else(|| root.join("output/firmwares"));
    emit(
        "catalog",
        json!({"firmwares":catalog(&root.join("firmware"))}),
    )?;
    let requested = args.required("firmware")?.canonicalize()?;
    let mut state = state::select(&requested, &library, args.path("state").as_deref())?;
    let mut donors = args.paths("cache-donor-root");
    donors.push(library.clone());
    // Modules compiled ahead of time for this engine, in a library of their
    // own: a firmware never run before starts with the code it shares.
    donors.push(root.join("output/dsp-donors"));
    cache::seed(root, &options.engine, &state, &donors)?;
    selected(&state, Some(&requested), false)?;
    let media_root = root.join("output/media");
    util::owned(&media_root)?;
    fs::create_dir_all(&media_root)?;
    let mut image = args.path("usb-image");
    let mut usb = Value::Null;
    if image.is_none() {
        if let Ok(saved) = read_json(&state.directory.join("shell.json")) {
            if let Some(p) = saved["image"].as_str() {
                let p = PathBuf::from(p);
                if p.is_file() {
                    image = Some(p);
                    usb = saved["usb"].clone();
                }
            }
        }
    }
    let defaults = root.join("output/desktop-defaults.json");
    if image.is_none() && defaults.is_file() {
        let defaults = read_json(&defaults)?;
        if let Some(path) = defaults["usb_image"].as_str() {
            let path = PathBuf::from(path)
                .canonicalize()
                .context("Default USB image is missing")?;
            ensure!(path.is_file(), "Default USB image must be a file");
            image = Some(path);
        }
    }
    if image.is_none() {
        let demo = media_root.join("demo.img");
        if !demo.exists() {
            media::tone_image(&demo, 30)?;
        }
        image = Some(demo);
    }
    if let Some(diagnostics) = args.path("diagnostics") {
        util::owned(&diagnostics)?;
        fs::create_dir_all(diagnostics)?;
    }
    let mut validate = args.flag("validate");
    let mut active = true;
    while active && !crate::stopped() {
        if validate {
            validate = false;
            let mut validation_args = args.clone();
            validation_args.flags.insert("force".into());
            if validation_args.value("mp3-image").is_none() {
                if let Some(image) = &image {
                    validation_args.values.insert(
                        "mp3-image".into(),
                        vec![image.to_string_lossy().into_owned()],
                    );
                }
            }
            let result =
                crate::validation::prepare(&validation_args, root, &state, &options, |detail| {
                    let mut cancelled = crate::stopped();
                    while let Ok(message) = receive.try_recv() {
                        match message["action"].as_str() {
                            Some("quit" | "stop") => {
                                cancelled = true;
                                active = false;
                            }
                            Some("diagnostic") => cancelled = true,
                            _ => pending.push_back(message),
                        }
                    }
                    emit(
                        "preparing",
                        json!({"message":detail["message"],"detail":detail}),
                    )?;
                    ensure!(!cancelled, "Validation cancelled");
                    Ok(())
                });
            match result {
                Ok(marker) => emit(
                    "prepared",
                    json!({"status":marker["status"],"message":"Playback check completed; hardware fidelity remains unverified"}),
                )?,
                Err(e) => {
                    emit(
                        "prepared",
                        json!({"status":"unqualified","message":format!("Validation failed: {e:#}. Starting player")}),
                    )?;
                }
            }
            if !active || crate::stopped() {
                break;
            }
        }
        let identity = cache::identity(root, &options.engine, &state)?;
        let prepared = cache::status(&state, &identity);
        options.mode = if prepared.is_some() {
            "prepared"
        } else {
            args.value("compile-mode").unwrap_or("adaptive")
        }
        .into();
        options.extra.clear();
        let qemu_img = options.engine.with_file_name("qemu-img");
        if let Some(image) = &image {
            let _lease = Lease::acquire(&state.directory)?;
            let overlay = media::overlay(image, &state, &qemu_img)?;
            options.extra.extend(media::options(&overlay));
        }
        let display = tempfile::Builder::new()
            .prefix("xdj-rust-display-")
            .tempdir_in("/tmp")?;
        let vnc = display.path().join("display.sock");
        options
            .extra
            .extend(["-vnc".into(), format!("unix:{}", vnc.display())]);
        let mut runtime = Runtime::start(&state, &options)?;
        let qmp = runtime.qmp();
        if args.path("diagnostics").is_some() {
            qmp.set("xdj-performance-enabled", json!(true))?;
        }
        let properties = qmp.execute("qom-list", json!({"path":"/machine"}))?;
        let has = |name: &str| {
            properties
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v["name"] == name))
        };
        let panel = has("xdj-panel-frame");
        let live = has("xdj-live-state");
        atomic_json(
            &library.join("selection.json"),
            &json!({"sha256":state.firmware_sha}),
        )?;
        emit(
            "running",
            json!({"vnc":vnc,"state":state.directory,"firmware":state.info,"source":state.source,"media":image,"usb":usb,"message":"Firmware running","compile_mode":options.mode,"preparation":prepared,"backend":"rust"}),
        )?;
        let mut controls = Controls::default();
        let mut lamps = Value::Null;
        let mut last = Instant::now() - Duration::from_secs(2);
        let mut restart = false;
        while active && !restart && !crate::stopped() && runtime.alive()? {
            let command = pending.pop_front().or_else(|| {
                receive
                    .recv_timeout(Duration::from_millis(if panel { 40 } else { 200 }))
                    .ok()
            });
            if let Some(command) = command {
                let kind = command["action"].as_str().unwrap_or("");
                let result = (|| -> Result<()> {
                    match kind {
                        "button" | "touch" | "pitch" | "jog" | "vinyl-speed" | "release"
                        | "mute" => {
                            controls.apply(runtime.qmp(), &command)?;
                            if kind == "mute" {
                                options.muted = command["muted"].as_bool().unwrap();
                            }
                        }
                        "quit" | "stop" => {
                            active = false;
                        }
                        "link" => {
                            ensure!(options.link.is_some(), "Start with a Link endpoint first");
                            let connected = command["connected"]
                                .as_bool()
                                .context("Missing connected boolean")?;
                            runtime
                                .qmp()
                                .execute("set_link", json!({"name":"ether","up":connected}))?;
                            emit("link", json!({"connected":connected}))?;
                        }
                        "restart" | "diagnostic" => {
                            restart = true;
                        }
                        "validate" => {
                            validate = true;
                            restart = true;
                        }
                        "firmware" => {
                            let requested = path(&command, "path")?.canonicalize()?;
                            let (_, _, info) = crate::firmware::load(&requested)?;
                            if info["sha256"] == state.firmware_sha {
                                selected(&state, Some(&requested), true)?;
                            } else {
                                let candidate = state::select(&requested, &library, None)?;
                                if let Some(image) = &image {
                                    let _lease = Lease::acquire(&candidate.directory)?;
                                    media::overlay(image, &candidate, &qemu_img)?;
                                }
                                cache::seed(root, &options.engine, &candidate, &donors)?;
                                atomic_json(
                                    &candidate.directory.join("shell.json"),
                                    &json!({"image":image,"usb":usb}),
                                )?;
                                controls.release(runtime.qmp())?;
                                state = candidate;
                                selected(&state, Some(&requested), false)?;
                                restart = true;
                            }
                        }
                        "music" | "export" | "image" => {
                            emit("loading", json!({"message":"Copying virtual USB…"}))?;
                            let mut report = Value::Null;
                            let candidate = if kind == "image" {
                                path(&command, "path")?.canonicalize()?
                            } else {
                                let target = media_root.join(format!(
                                    "{}.img",
                                    tempfile::Builder::new()
                                        .prefix("usb-")
                                        .tempdir_in(&media_root)?
                                        .path()
                                        .file_name()
                                        .unwrap()
                                        .to_string_lossy()
                                ));
                                if kind == "music" {
                                    let paths = command["paths"]
                                        .as_array()
                                        .context("Missing music paths")?
                                        .iter()
                                        .map(|p| {
                                            p.as_str()
                                                .map(PathBuf::from)
                                                .context("Invalid music path")
                                        })
                                        .collect::<Result<Vec<_>>>()?;
                                    media::music_image(&paths, &target)?;
                                } else {
                                    report = media::export_image(
                                        &path(&command, "folder")?,
                                        &target,
                                        command["audio_root"].as_str().map(Path::new),
                                    )?;
                                }
                                target
                            };
                            // QEMU holds the state lease; this serialized handler only creates a new collection.
                            media::overlay(&candidate, &state, &qemu_img)?;
                            atomic_json(
                                &state.directory.join("shell.json"),
                                &json!({"image":candidate,"usb":report}),
                            )?;
                            image = Some(candidate);
                            usb = report;
                            restart = true;
                        }
                        "capture" => {
                            let diagnostics = args
                                .path("diagnostics")
                                .context("Captures require --diagnostics")?;
                            let name = command["name"].as_str().unwrap_or("capture");
                            ensure!(
                                !name.is_empty()
                                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                                "Invalid capture name"
                            );
                            let dest = diagnostics.canonicalize()?.join(format!("{name}.wav"));
                            util::regular(&dest)?;
                            ensure!(!dest.exists(), "Capture already exists");
                            runtime.qmp().execute("human-monitor-command",json!({"command-line":format!("wavcapture \"{}\" xdj-audio 44100 16 2",dest.display())}))?;
                            emit("capturing", json!({"name":name}))?;
                        }
                        "capture-stop" => {
                            ensure!(
                                args.path("diagnostics").is_some(),
                                "Captures require --diagnostics"
                            );
                            runtime.qmp().execute(
                                "human-monitor-command",
                                json!({"command-line":"stopcapture 0"}),
                            )?;
                            emit("captured", json!({}))?;
                        }
                        "screenshot" => {
                            let dir = args
                                .path("diagnostics")
                                .context("Screenshots require --diagnostics")?;
                            let destination = dir.canonicalize()?.join("guest.ppm");
                            util::regular(&destination)?;
                            runtime
                                .qmp()
                                .execute("screendump", json!({"filename":destination}))?;
                        }
                        "read-memory" => {
                            let directory = args
                                .path("diagnostics")
                                .context("Memory capture requires --diagnostics")?;
                            let (address, size, destination) =
                                memory_capture(&command, &directory)?;
                            runtime.qmp().execute(
                                "pmemsave",
                                json!({"val":address,"size":size,"filename":destination}),
                            )?;
                            emit(
                                "memory-captured",
                                json!({"address":address,"size":size,"path":destination}),
                            )?;
                        }
                        "registers" => {
                            ensure!(
                                args.path("diagnostics").is_some(),
                                "Register capture requires --diagnostics"
                            );
                            let registers = runtime.qmp().execute(
                                "human-monitor-command",
                                json!({"command-line":"info registers"}),
                            )?;
                            emit("registers", json!({"registers":registers}))?;
                        }
                        "invalid" => bail!("{}", command["reason"]),
                        _ => bail!("Unknown shell action"),
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    emit(
                        "error",
                        json!({"message":format!("{e:#}"),"recoverable":true,"requested_source":if kind=="firmware"{command.get("path")}else{None}}),
                    )?;
                }
            }
            if restart || !active {
                break;
            }
            if panel {
                let raw = runtime.qmp().get("xdj-panel-frame")?;
                let report = lamps::decode(raw.as_str().unwrap_or("{}"));
                if report != lamps {
                    emit("lamps", report.clone())?;
                    lamps = report;
                }
            }
            if last.elapsed() >= Duration::from_secs(1) {
                if live {
                    let snapshot = runtime.qmp().object("xdj-live-state")?;
                    let mut stats = snapshot["audio"].clone();
                    stats["snapshot_age_us"] = snapshot["age_us"].clone();
                    emit("audio", stats)?;
                    if args.path("diagnostics").is_some() {
                        emit("performance", snapshot["performance"].clone())?;
                    }
                } else {
                    emit("audio", runtime.qmp().object("xdj-audio-state")?)?;
                    if args.path("diagnostics").is_some() {
                        emit(
                            "performance",
                            runtime.qmp().object("xdj-performance-state")?,
                        )?;
                    }
                }
                last = Instant::now();
            }
        }
        let _ = controls.release(runtime.qmp());
        if active && !restart && !crate::stopped() {
            ensure!(runtime.alive()?, "Player exited unexpectedly; see qemu.log");
        }
        drop(runtime);
        drop(display);
    }
    emit("stopped", json!({}))?;
    Ok(())
}
fn catalog(directory: &Path) -> Value {
    json!(xdj_catalog::archived(directory))
}

#[cfg(test)]
mod tests {
    use super::memory_capture;
    use serde_json::json;
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn memory_capture_bounds_guest_ranges_and_output_names() {
        let directory = tempfile::tempdir().unwrap();
        for (address, size) in [(0, 1), (0, 1024 * 1024), (u32::MAX as u64, 1)] {
            let (_, _, path) = memory_capture(
                &json!({"address":address,"size":size,"name":"scene-01"}),
                directory.path(),
            )
            .unwrap();
            assert_eq!(
                path,
                directory
                    .path()
                    .canonicalize()
                    .unwrap()
                    .join("scene-01.bin")
            );
        }
        for (address, size) in [
            (0, 0),
            (0, 1024 * 1024 + 1),
            (u32::MAX as u64, 2),
            (u64::MAX, 1),
        ] {
            assert!(
                memory_capture(
                    &json!({"address":address,"size":size,"name":"scene"}),
                    directory.path()
                )
                .is_err()
            );
        }
        for name in [
            "",
            "../escape",
            "nested/file",
            "a.bin",
            "a b",
            &"a".repeat(81),
        ] {
            assert!(
                memory_capture(&json!({"address":0,"size":1,"name":name}), directory.path())
                    .is_err()
            );
        }
        for message in [
            json!({"size":1,"name":"a"}),
            json!({"address":"0","size":1,"name":"a"}),
            json!({"address":-1,"size":1,"name":"a"}),
            json!({"address":0,"size":1.5,"name":"a"}),
        ] {
            assert!(memory_capture(&message, directory.path()).is_err());
        }
    }

    #[test]
    fn memory_capture_preserves_existing_files_and_rejects_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("scene.bin");
        let message = json!({"address":0,"size":1,"name":"scene"});
        fs::write(&destination, b"previous evidence").unwrap();
        assert!(memory_capture(&message, directory.path()).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"previous evidence");
        fs::remove_file(&destination).unwrap();
        symlink(directory.path().join("missing"), &destination).unwrap();
        assert!(memory_capture(&message, directory.path()).is_err());
    }
}
