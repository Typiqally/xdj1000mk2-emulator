use crate::rfb;
use anyhow::{Context, Result, ensure};
use std::{
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct Latest<T>(Mutex<Option<T>>);
impl<T> Default for Latest<T> {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}
impl<T> Latest<T> {
    pub fn offer(&self, value: T) {
        *self.0.lock().unwrap() = Some(value);
    }
    pub fn take(&self) -> Option<T> {
        self.0.lock().unwrap().take()
    }
}
pub struct Frame {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
    pub received: Instant,
    pub sequence: u64,
}
enum Socket {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
}
impl Socket {
    fn connect(endpoint: &str) -> Result<Self> {
        if let Some(address) = endpoint.strip_prefix("tcp:") {
            let address: SocketAddr = address.parse().context("Invalid display address")?;
            ensure!(
                address.ip().is_loopback(),
                "Display must be on this computer"
            );
            let s = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
            s.set_nodelay(true)?;
            Ok(Self::Tcp(s))
        } else {
            #[cfg(unix)]
            {
                Ok(Self::Unix(std::os::unix::net::UnixStream::connect(
                    endpoint.strip_prefix("unix:").unwrap_or(endpoint),
                )?))
            }
            #[cfg(not(unix))]
            {
                anyhow::bail!("This platform requires a local TCP display endpoint")
            }
        }
    }
    fn clone_socket(&self) -> Result<Self> {
        Ok(match self {
            Self::Tcp(s) => Self::Tcp(s.try_clone()?),
            #[cfg(unix)]
            Self::Unix(s) => Self::Unix(s.try_clone()?),
        })
    }
    fn shutdown(&self) {
        match self {
            Self::Tcp(s) => {
                let _ = s.shutdown(Shutdown::Both);
            }
            #[cfg(unix)]
            Self::Unix(s) => {
                let _ = s.shutdown(Shutdown::Both);
            }
        }
    }
}
impl Read for Socket {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(bytes),
            #[cfg(unix)]
            Self::Unix(s) => s.read(bytes),
        }
    }
}
impl Write for Socket {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.write(bytes),
            #[cfg(unix)]
            Self::Unix(s) => s.write(bytes),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.flush(),
            #[cfg(unix)]
            Self::Unix(s) => s.flush(),
        }
    }
}
pub struct Display {
    socket: Socket,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Display {
    pub fn start(
        endpoint: &str,
        latest: Arc<Latest<Frame>>,
        error: Arc<Mutex<Option<String>>>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self> {
        let socket = Socket::connect(endpoint)?;
        let mut reader = socket.clone_socket()?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let worker = thread::spawn(move || {
            let result = (|| -> Result<()> {
                let mut fb = rfb::handshake(&mut reader)?;
                let mut sequence = 0;
                while !cancel.load(Ordering::Relaxed) {
                    if rfb::update(&mut reader, &mut fb)? {
                        sequence += 1;
                        latest.offer(Frame {
                            size: [fb.width, fb.height],
                            rgba: fb.rgba().to_vec(),
                            received: Instant::now(),
                            sequence,
                        });
                        wake();
                    }
                }
                Ok(())
            })();
            if let Err(e) = result
                && !cancel.load(Ordering::Relaxed)
            {
                *error.lock().unwrap() = Some(format!("Firmware display: {e:#}"));
                wake();
            }
        });
        Ok(Self {
            socket,
            cancelled,
            worker: Some(worker),
        })
    }
}
impl Drop for Display {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.socket.shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
