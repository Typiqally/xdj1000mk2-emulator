use crate::{
    cache,
    cli::Args,
    controls::Controls,
    media,
    runtime::{Options, Runtime},
    state::{Lease, State},
    util::{self, atomic_json},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
    thread,
    time::{Duration, Instant},
};
const LOSS_KEYS: [&str; 3] = ["dropped_frames", "queue_recoveries", "serializer_underruns"];
pub fn monotonic_us() -> u64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes one initialized timespec through its valid pointer.
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t);
    }
    t.tv_sec as u64 * 1000000 + t.tv_nsec as u64 / 1000
}
pub fn screen_state(rows: &Value) -> Value {
    let mut remaining = None;
    let mut track = None;
    let mut titles = Vec::new();
    if let Some(rows) = rows.as_array() {
        for row in rows {
            let Some(text) = row["text"].as_str() else {
                continue;
            };
            let x = row["x"].as_f64().unwrap_or(0.);
            let y = row["y"].as_f64().unwrap_or(0.);
            if (0.13..0.22).contains(&x) && (0.73..0.81).contains(&y) {
                track = text.parse::<u32>().ok();
            }
            if (0.33..0.62).contains(&x) && (0.72..0.88).contains(&y) {
                let text = text.replace(' ', "");
                let parts: Vec<_> = text.split([':', '.', ',']).collect();
                if parts.len() == 3 && parts[1].len() == 2 && parts[2].len() == 3 {
                    if let (Ok(m), Ok(s), Ok(ms)) = (
                        parts[0].parse::<f64>(),
                        parts[1].parse::<f64>(),
                        parts[2].parse::<f64>(),
                    ) {
                        remaining = Some(m * 60. + s + ms / 1000.);
                    }
                }
            }
            if (0.12..0.85).contains(&x) && (0.12..0.23).contains(&y) {
                titles.push(text);
            }
        }
    }
    let title = titles.join(" ");
    let loaded =
        (!title.is_empty() && !title.to_lowercase().contains("not loaded") && !title.contains('['))
            || (track.is_some_and(|n| n > 0) && remaining.is_some_and(|n| n > 0.));
    json!({"remaining_seconds":remaining,"track_number":track,"title":title,"loaded":loaded,"text":rows})
}
pub fn host_bins(log: &str, start: u64, end: u64) -> Result<Value> {
    let mut origin = None;
    let mut bins = std::collections::BTreeMap::new();
    for line in log.lines() {
        if let Some((_, tail)) = line.split_once("xdj-output origin-us ") {
            origin = tail.trim().parse::<u64>().ok();
        }
        if let Some((_, tail)) = line.split_once("xdj-output second ") {
            let fields: Vec<_> = tail.split_whitespace().collect();
            if fields.len() >= 9 {
                let numbers = [0, 2, 4, 6, 8].map(|i| fields[i].parse::<u64>());
                if let [
                    Ok(second),
                    Ok(callbacks),
                    Ok(frames),
                    Ok(underruns),
                    Ok(missed),
                ] = numbers
                {
                    bins.insert(second, (callbacks, frames, underruns, missed));
                }
            }
        }
    }
    let origin = origin.context("CoreAudio callback statistics are missing")?;
    let first = start.saturating_sub(origin).div_ceil(1000000);
    let end = end.saturating_sub(origin) / 1000000;
    let selected: Vec<_> = (first..end).filter(|s| bins.contains_key(s)).collect();
    Ok(
        json!({"seconds":selected,"covered":!selected.is_empty()&&selected.len() as u64==end.saturating_sub(first),"frames":selected.iter().map(|s|bins[s].1).sum::<u64>(),"underruns":selected.iter().map(|s|bins[s].2).sum::<u64>(),"missed_frames":selected.iter().map(|s|bins[s].3).sum::<u64>()}),
    )
}
pub fn check_pcm(path: &Path, first: u64, end: u64, tone: bool) -> Result<Value> {
    ensure!(
        first <= end
            && end
                .checked_mul(8)
                .is_some_and(|n| n <= path.metadata().map(|m| m.len()).unwrap_or(0)),
        "Incomplete native PCM interval"
    );
    let mut input = fs::File::open(path)?;
    input.seek(SeekFrom::Start(first * 8))?;
    let mut peak = [0i32; 2];
    let mut crossings = [0u64; 2];
    let mut errors = [0u64; 2];
    let mut history = [[0i32; 2]; 2];
    let mut frames = 0u64;
    let coefficients = [
        2. * (2. * std::f64::consts::PI * 440. / 44100.).cos(),
        2. * (2. * std::f64::consts::PI * 880. / 44100.).cos(),
    ];
    let mut bytes = vec![0u8; 44100 * 8];
    let mut remaining = end - first;
    while remaining > 0 {
        let n = remaining.min(44100);
        input.read_exact(&mut bytes[..n as usize * 8])?;
        for frame in bytes[..n as usize * 8].chunks_exact(8) {
            for ch in 0..2 {
                let value = i32::from_le_bytes(frame[ch * 4..ch * 4 + 4].try_into().unwrap()) >> 16;
                peak[ch] = peak[ch].max(value.abs());
                if frames > 0 && history[ch][1] <= 0 && value > 0 {
                    crossings[ch] += 1;
                }
                if frames > 1
                    && (f64::from(value) - coefficients[ch] * f64::from(history[ch][1])
                        + f64::from(history[ch][0]))
                    .abs()
                        > 32.
                {
                    errors[ch] += 1;
                }
                history[ch] = [history[ch][1], value];
            }
            frames += 1;
        }
        remaining -= n;
    }
    let frequencies = crossings.map(|n| {
        if frames > 0 {
            n as f64 * 44100. / frames as f64
        } else {
            0.
        }
    });
    let continuous = frames > 0
        && if tone {
            errors == [0, 0]
                && peak.iter().all(|p| (5900..=6100).contains(p))
                && [440., 880.]
                    .iter()
                    .enumerate()
                    .all(|(i, target)| (frequencies[i] - target).abs() <= 1.1)
        } else {
            peak.iter().any(|p| *p > 0)
        };
    Ok(
        json!({"frames":frames,"seconds":frames as f64/44100.,"peak":peak,"frequencies":frequencies,"phase_errors":if tone{Some(errors)}else{None},"continuous":continuous,"continuity_verified":tone}),
    )
}
pub fn assess(report: &Value, minimum_seconds: f64) -> bool {
    report["first"]["running"] == true
        && report["last"]["running"] == true
        && report["last"]["stop"] == false
        && report["audio"]["continuous"] == true
        && report["audio"]["seconds"]
            .as_f64()
            .is_some_and(|s| s >= minimum_seconds)
        && report["native_time_advance_seconds"]
            .as_f64()
            .is_some_and(|s| s >= minimum_seconds - 3.)
        && report["host"]["covered"] == true
        && report["host"]["underruns"] == 0
        && report["host"]["missed_frames"] == 0
        && report["host"]["seconds"]
            .as_array()
            .is_some_and(|s| s.len() as f64 >= minimum_seconds - 2.)
        && LOSS_KEYS
            .iter()
            .all(|k| report["first"][k] == 0 && report["last"][k] == 0)
        && report["controls"]["queue_continuity"] == "pass"
        && LOSS_KEYS
            .iter()
            .all(|k| report["controls"]["first"][k] == 0)
}
fn wait(
    seconds: f64,
    progress: &mut impl FnMut(Value) -> Result<()>,
    session: u32,
    stage: &str,
) -> Result<()> {
    let begin = Instant::now();
    while begin.elapsed().as_secs_f64() < seconds {
        ensure!(!crate::stopped(), "Validation cancelled");
        progress(
            json!({"session":session,"stage":stage,"elapsed":begin.elapsed().as_secs(),"message":format!("Validating {session}/4 · {stage} · {}s",begin.elapsed().as_secs())}),
        )?;
        thread::sleep(Duration::from_millis(200).min(Duration::from_secs_f64(
            (seconds - begin.elapsed().as_secs_f64()).max(0.),
        )));
    }
    Ok(())
}
fn key(
    controls: &mut Controls,
    runtime: &mut Runtime,
    name: &str,
    wait: &mut impl FnMut(f64) -> Result<()>,
) -> Result<()> {
    controls.button(runtime.qmp(), name, true)?;
    wait(0.25)?;
    controls.button(runtime.qmp(), name, false)?;
    wait(0.5)
}
fn screen(runtime: &mut Runtime, directory: &Path, name: &str) -> Result<Value> {
    let ppm = directory.join(format!("{name}.ppm"));
    runtime
        .qmp()
        .execute("screendump", json!({"filename":ppm}))?;
    let rows = crate::ocr::read(&ppm)?;
    let state = screen_state(&rows);
    atomic_json(&directory.join(format!("{name}.json")), &state)?;
    Ok(state)
}
fn copy_code(source: &State, target: &State, identity: &Value) -> Result<()> {
    let _source = Lease::acquire(&source.directory)?;
    let _target = Lease::acquire(&target.directory)?;
    let from = source.directory.join("dsp-jit");
    let _ = cache::inventory(&from)?;
    let staging = tempfile::Builder::new()
        .prefix("dsp-jit-publish-")
        .tempdir_in(&target.directory)?;
    for batch in cache::batches(&from)? {
        let destination = staging.path().join(batch.file_name().unwrap());
        fs::create_dir(&destination)?;
        for name in cache::FILES {
            let p = batch.join(name);
            util::regular(&p)?;
            if p.exists() {
                fs::copy(p, destination.join(name))?;
            }
        }
    }
    let head = from.join("idle-head");
    if head.is_file() {
        util::regular(&head)?;
        fs::copy(head, staging.path().join("idle-head"))?;
    }
    atomic_json(&staging.path().join("identity.json"), identity)?;
    if from.join("preparation.json").is_file() {
        fs::copy(
            from.join("preparation.json"),
            staging.path().join("preparation.json"),
        )?;
    }
    let target = target.directory.join("dsp-jit");
    util::owned(&target)?;
    if target.exists() {
        let retired = cache::unique_directory(target.parent().unwrap(), "dsp-jit-retired-")?;
        fs::remove_dir(&retired)?;
        fs::rename(&target, retired)?;
    }
    fs::rename(staging.path(), target)?;
    Ok(())
}
pub fn prepare(
    args: &Args,
    root: &Path,
    state: &State,
    options: &Options,
    mut progress: impl FnMut(Value) -> Result<()>,
) -> Result<Value> {
    let identity = cache::identity(root, &options.engine, state)?;
    if !args.flag("force") {
        if let Some(marker) = cache::status(state, &identity) {
            return Ok(marker);
        }
    }
    let mp3 = args
        .required("mp3-image")
        .context("Validation needs --mp3-image pointing to a user-supplied MP3 USB image")?
        .canonicalize()?;
    let work = cache::unique_directory(&root.join("output"), "validation-")?;
    let _lease = Lease::acquire(&work)?;
    let wave = if let Some(path) = args.path("wav-image") {
        path.canonicalize()?
    } else {
        let p = work.join("tone.img");
        media::tone_image(&p, 900)?;
        p
    };
    let mut aggregate = State::prepare(&state.source, &work.join("aggregate"))?;
    let _ = copy_code(state, &aggregate, &identity);
    let seconds = args.number("seconds", 65)?;
    ensure!(
        seconds > 0 && seconds <= 3600,
        "Validation capture must be 1–3600 seconds"
    );
    let boot = args.number("boot-seconds", 45)?;
    let warm = args.number("warm-seconds", 150)?;
    let mut coverage = Vec::new();
    let mut all_pass = true;
    for session in 1..=4 {
        let prepared = session > 2;
        let tone = session % 2 == 1;
        let image = if tone { &wave } else { &mp3 };
        let directory = work.join(format!("session-{session}"));
        fs::create_dir(&directory)?;
        let test_state = State::prepare(&state.source, &directory.join("state"))?;
        let _ = copy_code(&aggregate, &test_state, &identity);
        let overlay = {
            let _lease = Lease::acquire(&test_state.directory)?;
            media::overlay(
                image,
                &test_state,
                &options.engine.with_file_name("qemu-img"),
            )?
        };
        let mut options = options.clone();
        options.muted = true;
        options.audio = "coreaudio".into();
        options.mode = if prepared { "prepared" } else { "adaptive" }.into();
        options.lag_cap_ms = 0;
        options.capture = Some(directory.clone());
        options.extra = media::options(&overlay);
        progress(
            json!({"session":session,"stage":"boot","message":format!("Validating {session}/4 · {}",if tone{"PCM tone"}else{"MP3 USB"})}),
        )?;
        let mut runtime = Runtime::start(&test_state, &options)?;
        let mut controls = Controls::default();
        let run_result = (|| -> Result<Value> {
            let mut pause = |s| wait(s, &mut progress, session, "playback");
            pause(boot as f64)?;
            screen(&mut runtime, &directory, "boot")?;
            controls.touch(runtime.qmp(), 54, 260, true)?;
            pause(0.3)?;
            controls.touch(runtime.qmp(), 54, 260, false)?;
            pause(5.)?;
            screen(&mut runtime, &directory, "browse")?;
            if !tone {
                key(&mut controls, &mut runtime, "browse-down", &mut pause)?;
                key(&mut controls, &mut runtime, "browse-down", &mut pause)?;
                key(&mut controls, &mut runtime, "load", &mut pause)?;
                pause(3.)?;
            }
            key(&mut controls, &mut runtime, "load", &mut pause)?;
            pause(3.)?;
            key(&mut controls, &mut runtime, "load", &mut pause)?;
            pause(8.)?;
            let loaded = screen(&mut runtime, &directory, "loaded")?;
            ensure!(
                loaded["loaded"] == true,
                "No native track loaded; inspect screenshots"
            );
            pause(3.)?;
            let progressed = screen(&mut runtime, &directory, "start-check")?;
            if loaded["remaining_seconds"] == progressed["remaining_seconds"] {
                key(&mut controls, &mut runtime, "play", &mut pause)?;
                pause(4.)?;
            }
            let control_first = runtime.qmp().object("xdj-audio-state")?;
            controls.touch(runtime.qmp(), 720, 40, true)?;
            pause(0.2)?;
            controls.touch(runtime.qmp(), 720, 40, false)?;
            pause(2.)?;
            screen(&mut runtime, &directory, "perform")?;
            for (name, delay) in [
                ("play", 2.),
                ("cue", 0.),
                ("play", 2.),
                ("loop-in", 2.),
                ("loop-out", 3.),
                ("reloop", 0.),
            ] {
                key(&mut controls, &mut runtime, name, &mut pause)?;
                pause(delay)?;
            }
            controls.apply(runtime.qmp(), &json!({"action":"pitch","value":48000}))?;
            pause(2.)?;
            controls.apply(runtime.qmp(), &json!({"action":"pitch","value":32768}))?;
            controls.button(runtime.qmp(), "jog-touch", true)?;
            controls.apply(runtime.qmp(), &json!({"action":"jog","ticks":32}))?;
            pause(1.)?;
            controls.button(runtime.qmp(), "jog-touch", false)?;
            for (name, delay) in [
                ("reverse", 1.),
                ("reverse", 0.),
                ("master-tempo", 2.),
                ("master-tempo", 0.),
                ("shortcut", 2.),
                ("back", 0.),
            ] {
                key(&mut controls, &mut runtime, name, &mut pause)?;
                pause(delay)?;
            }
            controls.release(runtime.qmp())?;
            let control_last = runtime.qmp().object("xdj-audio-state")?;
            pause(if prepared { 5. } else { warm as f64 })?;
            let before = screen(&mut runtime, &directory, "warmed")?;
            let first = runtime.qmp().object("xdj-audio-state")?;
            let start_us = monotonic_us();
            pause(seconds as f64)?;
            let end_us = monotonic_us();
            let last = runtime.qmp().object("xdj-audio-state")?;
            let after = screen(&mut runtime, &directory, "advanced")?;
            let controls = json!({"profile":3,"exercised":cache::CONTROL_NAMES,"first":control_first,"last":control_last,"queue_continuity":if LOSS_KEYS.iter().all(|k|control_first[k]==control_last[k])&&control_last["running"]==true&&control_last["stop"]==false{"pass"}else{"fail"},"semantics":"unverified"});
            Ok(
                json!({"first":first,"last":last,"controls":controls,"start_us":start_us,"end_us":end_us,"native_time_advance_seconds":before["remaining_seconds"].as_f64().zip(after["remaining_seconds"].as_f64()).map(|(a,b)|a-b),"identity":identity,"media_sha256":util::file_hash(image)?,"factory_state":true,"mode":if tone{"wav"}else{"export"},"compilation_mode":options.mode,"hardware_reference":"unverified","controls_semantics":"unverified"}),
            )
        })();
        let _ = controls.release(runtime.qmp());
        drop(runtime);
        let mut report = match run_result {
            Ok(report) => report,
            Err(error) => {
                atomic_json(
                    &directory.join("report.json"),
                    &json!({"status":"unverified","error":format!("{error:#}"),"identity":identity}),
                )?;
                return Err(error);
            }
        };
        report["audio"] = check_pcm(
            &directory.join("consumed.s32le"),
            report["first"]["consumed_frames"]
                .as_u64()
                .context("Missing consumed frame counter")?,
            report["last"]["consumed_frames"]
                .as_u64()
                .context("Missing consumed frame counter")?,
            tone,
        )?;
        report["host"] = host_bins(
            &fs::read_to_string(test_state.directory.join("qemu.log"))?,
            report["start_us"].as_u64().unwrap(),
            report["end_us"].as_u64().unwrap(),
        )?;
        let passed = seconds >= 60 && assess(&report, 60.);
        report["status"] = json!(if passed { "pass" } else { "fail" });
        atomic_json(&directory.join("report.json"), &report)?;
        copy_code(&test_state, &aggregate, &identity)?;
        if prepared {
            all_pass &= passed;
            coverage.push(json!({"mode":report["mode"],"identity":identity,"seconds":report["audio"]["seconds"],"controls":report["controls"],"run_directory":directory}));
        }
        aggregate = State::prepare(&state.source, &work.join("aggregate"))?;
    }
    let marker = json!({"schema":1,"profile":3,"status":if all_pass{"ready"}else{"unqualified"},"identity":identity,"modules":cache::inventory(&aggregate.directory.join("dsp-jit"))?,"coverage":coverage,"hardware_reference":"unverified","correctness":"unverified","report_directory":work});
    {
        let _lease = Lease::acquire(&aggregate.directory)?;
        atomic_json(
            &aggregate.directory.join("dsp-jit/preparation.json"),
            &marker,
        )?;
    }
    copy_code(&aggregate, state, &identity)?;
    Ok(marker)
}
