use crate::{
    cache,
    qmp::Qmp,
    state::{Lease, State},
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    fs::File,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct Options {
    pub root: PathBuf,
    pub engine: PathBuf,
    pub data: PathBuf,
    pub python: PathBuf,
    pub audio: String,
    pub muted: bool,
    pub mode: String,
    pub lag_cap_ms: u64,
    pub capture: Option<PathBuf>,
    pub link: Option<crate::link::Link>,
    pub extra: Vec<String>,
}
impl Options {
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.into(),
            engine: root.join("output/engine/build/qemu-system-sh4"),
            data: root.join("output/engine/qemu-9.1.0/pc-bios"),
            python: "python3".into(),
            audio: "coreaudio".into(),
            muted: true,
            mode: "adaptive".into(),
            lag_cap_ms: 300,
            capture: None,
            link: None,
            extra: Vec::new(),
        }
    }
}
pub struct Runtime {
    pub qmp: Option<Qmp>,
    child: Child,
    _lease: Lease,
}
impl Runtime {
    pub fn start(state: &State, options: &Options) -> Result<Self> {
        ensure!(
            ["adaptive", "prepared", "interpreter"].contains(&options.mode.as_str()),
            "Invalid compilation mode"
        );
        ensure!(
            ["coreaudio", "off", "wav"].contains(&options.audio.as_str()),
            "Invalid audio backend"
        );
        let lease = Lease::acquire(&state.directory)?;
        let identity = cache::identity(&options.root, &options.engine, state)?;
        let cache = cache::prepare(state, &identity)?;
        let log = File::create(state.directory.join("qemu.log"))?;
        let mut command = Command::new(&options.engine);
        for (k, _) in std::env::vars_os() {
            let s = k.to_string_lossy();
            if s.starts_with("CDJ_") || s.starts_with("C66X_") || s.starts_with("GUI_") {
                command.env_remove(k);
            }
        }
        let audio = match options.audio.as_str() {
            "off" => "none,id=xdj-audio".to_owned(),
            "wav" => format!(
                "wav,id=xdj-audio,path={},out.frequency=44100",
                escape(&state.directory.join("audio.wav"))
            ),
            _ => "coreaudio,id=xdj-audio".into(),
        };
        command
            .args(["-L"])
            .arg(&options.data)
            .args(["-machine", "xdj1000mk2", "-accel", "tcg", "-drive"])
            .arg(format!(
                "if=pflash,format=raw,file={}",
                escape(&state.directory.join("flash.bin"))
            ))
            .args([
                "-display",
                "none",
                "-audiodev",
                &audio,
                "-qmp",
                "stdio",
                "-nic",
                "none",
                "-monitor",
                "none",
            ])
            .args(&options.extra);
        if let Some(link) = &options.link {
            link.configure(&mut command, &state.directory);
        }
        if options.muted {
            command.arg("-S");
        }
        if let Some(directory) = &options.capture {
            for name in ["produced.s32le", "consumed.s32le"] {
                crate::util::regular(&directory.join(name))?;
            }
            command
                .env("CDJ_XDJ_PCM_CAPTURE", directory.join("produced.s32le"))
                .env("CDJ_XDJ_PCM_CONSUMED", directory.join("consumed.s32le"));
        }
        if options.mode == "adaptive" {
            let generator = cache::generator(&options.root, &options.engine);
            command
                .env("C66X_JIT_AUTO", &cache)
                .env("C66X_JIT_CC", "cc")
                .env("C66X_JIT_PYTHON", &options.python)
                .env("C66X_JIT_GEN", generator.script)
                .env("C66X_JIT_LIB", generator.library)
                .env("C66X_JIT_AUTO_FN", "1500")
                .env("C66X_JIT_AUTO_ROOTS", "400")
                .env("C66X_JIT_AUTO_COLD", "0")
                .env("C66X_JIT_AUTO_NODES", "1200")
                .env("C66X_JIT_AUTO_LOOPS", "32");
        } else if options.mode == "prepared" {
            let paths: Vec<PathBuf> = cache::batches(&cache)?
                .iter()
                .map(|b| b.join("m.so"))
                .collect();
            ensure!(!paths.is_empty(), "No prepared DSP modules");
            command.env("C66X_JIT", std::env::join_paths(paths)?);
        }
        if options.mode != "interpreter" {
            command.env("CDJ_XDJ_DSP_IDLE_HINT", cache.join("idle-head"));
        }
        if options.lag_cap_ms > 0 {
            command.env("CDJ_XDJ_DSP_LAG_CAP_MS", options.lag_cap_ms.to_string());
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .process_group(0);
        let child = command.spawn().context("Launch QEMU")?;
        let mut runtime = Self {
            qmp: None,
            child,
            _lease: lease,
        };
        let reader = runtime.child.stdout.take().context("QMP stdout")?;
        let writer = runtime.child.stdin.take().context("QMP stdin")?;
        let init = Qmp::connect(reader, writer);
        match init {
            Ok(mut qmp) => {
                if options.muted {
                    qmp.set("xdj-audio-muted", json!(true))?;
                    qmp.execute("cont", json!({}))?;
                }
                qmp.timeout = Duration::from_secs(120);
                runtime.qmp = Some(qmp);
            }
            Err(e) => {
                let log =
                    std::fs::read_to_string(state.directory.join("qemu.log")).unwrap_or_default();
                let detail: String = log.chars().take(4096).collect();
                return Err(e).with_context(|| {
                    format!(
                        "QEMU startup failed: {detail}; see {}",
                        state.directory.join("qemu.log").display()
                    )
                });
            }
        }
        Ok(runtime)
    }
    pub fn qmp(&mut self) -> &mut Qmp {
        self.qmp.as_mut().expect("Initialized runtime")
    }
    pub fn alive(&mut self) -> Result<bool> {
        Ok(self.child.try_wait()?.is_none())
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(qmp) = &mut self.qmp {
            qmp.timeout = Duration::from_secs(2);
            let _ = qmp.execute("quit", json!({}));
        }
        self.qmp.take();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub fn escape(path: &Path) -> String {
    path.to_string_lossy().replace(',', ",,")
}
