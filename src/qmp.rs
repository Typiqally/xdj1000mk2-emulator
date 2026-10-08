use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, Read, Write},
    os::fd::AsRawFd,
    time::{Duration, Instant},
};
pub const MAX_REPLY: usize = 16 * 1024 * 1024;
pub fn bounded_line(reader: &mut impl BufRead, limit: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(out);
        }
        let n = buffer
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(buffer.len());
        ensure!(out.len() + n <= limit, "Protocol line exceeds size limit");
        let complete = buffer[n - 1] == b'\n';
        out.extend_from_slice(&buffer[..n]);
        reader.consume(n);
        if complete {
            return Ok(out);
        }
    }
}
trait ReplyStream: Read + Send + AsRawFd {}
impl<T: Read + Send + AsRawFd> ReplyStream for T {}
pub struct Qmp {
    reader: Box<dyn ReplyStream>,
    writer: Box<dyn Write + Send>,
    buffer: Vec<u8>,
    sequence: u64,
    pub timeout: Duration,
}
impl Qmp {
    pub fn connect(
        reader: impl Read + Send + AsRawFd + 'static,
        writer: impl Write + Send + 'static,
    ) -> Result<Self> {
        let mut qmp = Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            buffer: Vec::new(),
            sequence: 0,
            timeout: Duration::from_secs(15),
        };
        let greeting = qmp.receive(Instant::now() + qmp.timeout)?;
        ensure!(greeting.get("QMP").is_some(), "Invalid QMP greeting");
        qmp.execute("qmp_capabilities", json!({}))?;
        Ok(qmp)
    }
    fn receive(&mut self, deadline: Instant) -> Result<Value> {
        loop {
            if let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
                ensure!(end <= MAX_REPLY, "QMP reply exceeds size limit");
                let reply = serde_json::from_slice(&self.buffer[..end])?;
                self.buffer.drain(..=end);
                return Ok(reply);
            }
            ensure!(
                self.buffer.len() <= MAX_REPLY,
                "QMP reply exceeds size limit"
            );
            let remaining = deadline.saturating_duration_since(Instant::now());
            ensure!(!remaining.is_zero(), "QMP reply timed out");
            let mut descriptor = libc::pollfd {
                fd: self.reader.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: poll receives exactly one initialized descriptor for the live owned reader.
            let status = unsafe {
                libc::poll(
                    &mut descriptor,
                    1,
                    remaining.as_millis().clamp(1, 250) as i32,
                )
            };
            if status < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    ensure!(!crate::stopped(), "QMP cancelled");
                    continue;
                }
                return Err(error.into());
            }
            if status == 0 {
                ensure!(!crate::stopped(), "QMP cancelled");
                continue;
            }
            let mut chunk = [0u8; 65536];
            let n = self.reader.read(&mut chunk)?;
            ensure!(n > 0, "QMP connection closed");
            self.buffer.extend_from_slice(&chunk[..n]);
        }
    }
    pub fn execute(&mut self, name: &str, arguments: Value) -> Result<Value> {
        self.sequence += 1;
        serde_json::to_writer(
            &mut self.writer,
            &json!({"execute":name,"arguments":arguments,"id":self.sequence}),
        )?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        let deadline = Instant::now() + self.timeout;
        loop {
            let reply = self.receive(deadline)?;
            if reply["id"].as_u64() != Some(self.sequence) {
                continue;
            }
            if let Some(error) = reply.get("error") {
                bail!("QMP {name}: {}", error["desc"]);
            }
            return reply.get("return").cloned().context("Missing QMP result");
        }
    }
    pub fn get(&mut self, name: &str) -> Result<Value> {
        self.execute("qom-get", json!({"path":"/machine","property":name}))
    }
    pub fn set(&mut self, name: &str, value: Value) -> Result<Value> {
        self.execute(
            "qom-set",
            json!({"path":"/machine","property":name,"value":value}),
        )
    }
    pub fn object(&mut self, name: &str) -> Result<Value> {
        let v = self.get(name)?;
        match v.as_str() {
            Some(s) => Ok(serde_json::from_str(s)?),
            None => Ok(v),
        }
    }
}
