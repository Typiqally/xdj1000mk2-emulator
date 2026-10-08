//! Synthetic local display/JSON peer for protocol tests and repeatable UI benchmarks.
//! This does not emulate firmware, DSP or audio.
use anyhow::Result;
use serde_json::{Value, json};
#[cfg(not(unix))]
use std::net::{TcpListener as Listener, TcpStream as Stream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};
use std::{
    io::{Read, Write},
    net::Shutdown,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
fn send(event: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{event}");
    let _ = out.flush();
}
fn serve(mut socket: Stream, stop: &AtomicBool, frames: &AtomicU64, pixels: &[u8]) -> Result<()> {
    socket.write_all(b"RFB 003.008\n")?;
    let mut version = [0; 12];
    socket.read_exact(&mut version)?;
    socket.write_all(&[1, 1])?;
    let mut one = [0];
    socket.read_exact(&mut one)?;
    socket.write_all(&[0; 4])?;
    socket.read_exact(&mut one)?;
    let mut init = [0; 24];
    init[..2].copy_from_slice(&800u16.to_be_bytes());
    init[2..4].copy_from_slice(&480u16.to_be_bytes());
    socket.write_all(&init)?;
    let mut format = [0; 20];
    socket.read_exact(&mut format)?;
    let mut encodings = [0; 12];
    socket.read_exact(&mut encodings)?;
    let started = Instant::now();
    let mut next = Instant::now();
    let mut local = 0;
    let sparse = std::env::var("XDJ_FIXTURE_DAMAGE").is_ok_and(|s| s == "small");
    while !stop.load(Ordering::Relaxed) {
        let mut request = [0; 10];
        socket.read_exact(&mut request)?;
        if let Some(delay) = next.checked_duration_since(Instant::now()) {
            thread::sleep(delay);
        }
        if sparse && local > 0 {
            // Same 60 Hz schedule with 1% LCD damage after the initial frame.
            socket.write_all(&[0, 0, 0, 1, 0, 100, 0, 100, 0, 80, 0, 48, 0, 0, 0, 0])?;
            for row in 100..148 {
                socket.write_all(&pixels[(row * 800 + 100) * 4..(row * 800 + 180) * 4])?;
            }
        } else {
            // Repeated full frames isolate worst-case upload/presentation cost at 60 Hz.
            socket.write_all(&[0, 0, 0, 1, 0, 0, 0, 0, 3, 32, 1, 224, 0, 0, 0, 0])?;
            socket.write_all(pixels)?;
        }
        socket.flush()?;
        local += 1;
        frames.fetch_add(1, Ordering::Relaxed);
        next = started + Duration::from_secs_f64(local as f64 / 60.);
    }
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str| {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
    };
    let dir = tempfile::Builder::new()
        .prefix("xdj-ui-fixture-")
        .tempdir()?;
    #[cfg(unix)]
    let (listener, endpoint) = {
        let path = dir.path().join("vnc.sock");
        (Listener::bind(&path)?, path.display().to_string())
    };
    #[cfg(not(unix))]
    let (listener, endpoint) = {
        let l = Listener::bind("127.0.0.1:0")?;
        let p = format!("tcp:{}", l.local_addr()?);
        (l, p)
    };
    listener.set_nonblocking(true)?;
    let stopped = Arc::new(AtomicBool::new(false));
    let frames = Arc::new(AtomicU64::new(0));
    let live = Arc::new(Mutex::new(None::<Stream>));
    let mut pixels = vec![0; 800 * 480 * 4];
    for y in 0..480 {
        for x in 0..800 {
            let offset = (y * 800 + x) * 4;
            pixels[offset..offset + 4].copy_from_slice(&[
                (x % 256) as u8,
                (y % 256) as u8,
                if (x / 40 + y / 40) % 2 == 0 { 220 } else { 20 },
                0,
            ]);
        }
    }
    if let Ok(path) = std::env::var("XDJ_FIXTURE_IMAGE") {
        let image = image::open(path)?
            .resize_exact(800, 480, image::imageops::FilterType::Nearest)
            .to_rgba8();
        for (out, input) in pixels
            .chunks_exact_mut(4)
            .zip(image.as_raw().chunks_exact(4))
        {
            out.copy_from_slice(&[input[2], input[1], input[0], 0]);
        }
    }
    let stop = stopped.clone();
    let count = frames.clone();
    let socket = live.clone();
    let worker = thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((client, _)) => {
                    if let Err(e) = client.set_nonblocking(false) {
                        eprintln!("Fixture socket: {e}");
                        continue;
                    }

                    *socket.lock().unwrap() = client.try_clone().ok();
                    if let Err(e) = serve(client, &stop, &count, &pixels)
                        && !stop.load(Ordering::Relaxed)
                    {
                        eprintln!("Fixture display: {e:#}");
                    }
                    *socket.lock().unwrap() = None;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(_) => break,
            }
        }
    });
    let requested =
        std::path::PathBuf::from(arg("--firmware").map(String::as_str).unwrap_or("fixture"));
    let requested = if requested.is_absolute() {
        requested
    } else {
        std::env::current_dir()?.join(requested)
    };
    send(
        json!({"event":"selected","requested_source":requested,"source":requested,"state":dir.path(),"firmware":{"version":"fixture","original":false,"sha256":"synthetic"},"unchanged":false}),
    );
    let started = Instant::now();
    send(
        json!({"event":"running","vnc":endpoint,"firmware":{"version":"fixture","original":false,"sha256":"synthetic"},"source":"fixture","state":dir.path(),"message":"Synthetic UI benchmark — no firmware or audio"}),
    );
    send(
        json!({"event":"lamps","lit":["play","cue","vinyl"],"jog":{"position":35},"usb":[50,100,255]}),
    );
    let mut commands = Vec::new();
    let mut stdin = std::io::stdin().lock();
    while let Some(line) = xdj_ui::bridge::read_line(&mut stdin, 65536)? {
        let command: Value = serde_json::from_slice(&line)?;
        commands.push(json!({"after":started.elapsed().as_secs_f64(),"command":command}));
        if command["action"] == "quit" {
            break;
        }
        if command["action"] == "restart" || command["action"] == "firmware" {
            send(json!({"event":"running","vnc":endpoint,"message":"Synthetic peer running"}));
        }
        if [
            "button",
            "touch",
            "pitch",
            "jog",
            "vinyl-speed",
            "release",
            "mute",
            "bench-echo",
        ]
        .iter()
        .any(|s| command["action"] == *s)
        {
            send(xdj_ui::timing::stamp(
                json!({"event":"input-ack","command":command,"peer_after":started.elapsed().as_secs_f64()}),
                true,
                "peer_received_unix_us",
            ));
        }
    }
    stopped.store(true, Ordering::Relaxed);
    if let Some(socket) = live.lock().unwrap().as_ref() {
        let _ = socket.shutdown(Shutdown::Both);
    }
    let _ = worker.join();
    if let Some(path) = arg("--diagnostics") {
        std::fs::create_dir_all(path)?;
        std::fs::write(std::path::Path::new(path).join("fixture.json"),json!({"frames":frames.load(Ordering::Relaxed),"elapsed":started.elapsed().as_secs_f64(),"commands":commands}).to_string())?;
    }
    drop(dir);
    Ok(())
}
