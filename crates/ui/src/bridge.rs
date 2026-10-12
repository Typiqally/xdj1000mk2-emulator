//! Owned JSON child bridge. All desktop I/O runs away from the render thread.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub fn read_line(input: &mut impl BufRead, limit: usize) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let length = input.take(limit as u64 + 1).read_until(b'\n', &mut bytes)?;
    if length == 0 {
        return Ok(None);
    }
    ensure!(
        length <= limit && bytes.last() == Some(&b'\n'),
        "Backend message exceeds limit or is truncated"
    );
    Ok(Some(bytes))
}
pub struct Bridge {
    pub events: Receiver<Value>,
    pub error: Arc<Mutex<Option<String>>>,
    commands: Option<SyncSender<Value>>,
    child: Child,
    workers: Vec<JoinHandle<()>>,
}
impl Bridge {
    pub fn spawn(
        executable: &Path,
        args: &[String],
        root: &Path,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self> {
        let mut child = Command::new(executable)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("Cannot start backend {}", executable.display()))?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (send, receive) = mpsc::sync_channel::<Value>(128);
        let (events, incoming) = mpsc::sync_channel(256);
        let error = Arc::new(Mutex::new(None));
        let fault = error.clone();
        let wake_writer = wake.clone();
        let writer = thread::spawn(move || {
            for command in receive {
                let result = (|| -> Result<()> {
                    serde_json::to_writer(&mut stdin, &command)?;
                    stdin.write_all(b"\n")?;
                    stdin.flush()?;
                    Ok(())
                })();
                if let Err(e) = result {
                    *fault.lock().unwrap() = Some(format!("Backend input: {e}"));
                    wake_writer();
                    break;
                }
                if command["action"] == "quit" {
                    break;
                }
            }
        });
        let mut workers = vec![writer];
        for (stream, source) in [
            (Box::new(stdout) as Box<dyn Read + Send>, "events"),
            (Box::new(stderr) as Box<dyn Read + Send>, "log"),
        ] {
            let send = events.clone();
            let fault = error.clone();
            let notify = wake.clone();
            workers.push(thread::spawn(move || {
                let result = (|| -> Result<()> {
                    let mut input = BufReader::new(stream);
                    while let Some(line) = read_line(&mut input, 1_048_576)? {
                        let event = if source == "events" {
                            let value: Value = serde_json::from_slice(&line)?;
                            ensure!(value.is_object(), "Invalid backend event");
                            value
                        } else {
                            json!({"event":"log","message":String::from_utf8_lossy(&line)})
                        };
                        // Occluded windows can stop polling for minutes. Bound
                        // memory with backpressure, without closing the pipe
                        // the running backend still writes its reports into.
                        send.send(event).context("Backend event queue is closed")?;
                        notify();
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    *fault.lock().unwrap() = Some(e.to_string());
                    notify();
                }
            }));
        }
        Ok(Self {
            events: incoming,
            error,
            commands: Some(send),
            child,
            workers,
        })
    }
    pub fn send(&self, value: Value) -> Result<()> {
        self.commands
            .as_ref()
            .context("Backend stopped")?
            .try_send(value)
            .context("Backend is busy; input was not sent")
    }
    pub fn exited(&mut self) -> Result<bool> {
        Ok(self.child.try_wait()?.is_some())
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        // Release readers blocked on a full queue before joining their threads.
        // The UI no longer consumes events once bridge shutdown starts.
        let (_, closed) = mpsc::channel();
        self.events = closed;
        if let Some(send) = self.commands.take() {
            let _ = send.try_send(json!({"action":"quit"}));
            drop(send);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            if Instant::now() >= deadline {
                #[cfg(unix)]
                {
                    // SAFETY: signal only our still-owned child; its handler tears down QEMU.
                    unsafe {
                        libc::kill(self.child.id() as i32, libc::SIGTERM);
                    }
                    let grace = Instant::now() + Duration::from_secs(2);
                    while Instant::now() < grace && self.child.try_wait().ok().flatten().is_none() {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
                if self.child.try_wait().ok().flatten().is_none() {
                    let _ = self.child.kill();
                }
                let _ = self.child.wait();
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}
