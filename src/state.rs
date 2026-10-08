use crate::{
    firmware,
    util::{self, atomic_json, hash, owned, read_json, regular},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

pub struct Lease {
    _file: File,
}
impl Lease {
    pub fn acquire(directory: &Path) -> Result<Self> {
        owned(directory)?;
        fs::create_dir_all(directory)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join(".run.lock"))?;
        let m = file.metadata()?;
        ensure!(
            m.is_file() && m.nlink() == 1,
            "Runtime lock must be an owned regular file"
        );
        // SAFETY: flock only operates on this live file descriptor; the file owns its lifetime.
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        ensure!(locked == 0, "This emulator state is already running");
        for name in ["qemu.log", "audio.wav"] {
            regular(&directory.join(name))?;
        }
        owned(&directory.join("dsp-jit"))?;
        if directory.join("dsp-jit").exists() {
            ensure!(
                directory.join("dsp-jit").is_dir(),
                "Invalid DSP cache directory"
            );
        }
        Ok(Self { _file: file })
    }
}
#[derive(Debug, Clone)]
pub struct State {
    pub directory: PathBuf,
    pub firmware_sha: String,
    pub source: PathBuf,
    pub info: Value,
}
impl State {
    pub fn prepare(source: &Path, directory: &Path) -> Result<Self> {
        let source = source.canonicalize()?;
        owned(directory)?;
        fs::create_dir_all(directory)?;
        let directory = directory.canonicalize()?;
        ensure!(
            source.parent() != Some(directory.as_path()),
            "State must be separate from source"
        );
        let (package, _, info) = firmware::load(&source)?;
        let firmware_sha = info["sha256"].as_str().unwrap().to_owned();
        let _lease = Lease::acquire(&directory)?;
        let flash = directory.join("flash.bin");
        let panel = directory.join("panel.bin");
        let manifest = directory.join("manifest.json");
        for p in [&flash, &panel, &manifest] {
            regular(p)?;
        }
        let flash_size = firmware::profile()["nor"]["size_bytes"].as_u64().unwrap();
        if [&flash, &panel, &manifest].iter().any(|p| p.exists()) {
            let saved = read_json(&manifest).context("Existing state is incomplete")?;
            ensure!(
                saved["schema"] == 1
                    && saved["source_sha256"] == firmware_sha
                    && saved["machine"] == "xdj1000mk2"
                    && saved["flash_size"] == flash_size
                    && fs::metadata(&flash)?.len() == flash_size
                    && util::file_hash(&panel)? == hash(&package.panel.memory)
                    && saved["panel_sha256"] == info["panel_sha256"],
                "State is incomplete or belongs to different firmware"
            );
        } else {
            let mut bytes = package.main.memory;
            bytes.resize(flash_size as usize, 0xff);
            util::exclusive(&flash)?.write_all(&bytes)?;
            util::exclusive(&panel)?.write_all(&package.panel.memory)?;
            atomic_json(
                &manifest,
                &json!({"schema":1,"source_sha256":firmware_sha,"machine":"xdj1000mk2","flash_size":flash_size,"panel_sha256":info["panel_sha256"]}),
            )?;
        }
        Ok(Self {
            directory,
            firmware_sha,
            source,
            info,
        })
    }
}
pub fn select(source: &Path, library: &Path, state_override: Option<&Path>) -> Result<State> {
    if let Some(directory) = state_override {
        let source = source.canonicalize()?;
        if directory.exists() {
            ensure!(
                directory.canonicalize()?.as_path() != source.parent().unwrap(),
                "State must be separate from source"
            );
        }
    }
    let (_, raw, info) = firmware::load(source)?;
    owned(library)?;
    fs::create_dir_all(library)?;
    let entry = library.join(info["sha256"].as_str().unwrap());
    owned(&entry)?;
    fs::create_dir_all(&entry)?;
    let imported = entry.join("firmware.UPD");
    {
        let _lease = Lease::acquire(&entry)?;
        regular(&imported)?;
        if imported.exists() {
            ensure!(
                util::file_hash(&imported)? == info["sha256"],
                "Imported firmware changed"
            );
        } else {
            let mut temporary = tempfile::NamedTempFile::new_in(&entry)?;
            temporary.write_all(&raw)?;
            temporary.as_file().sync_all()?;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o444))?;
            temporary
                .persist_noclobber(&imported)
                .map_err(|e| e.error)?;
        }
    }
    let directory = state_override
        .map(PathBuf::from)
        .unwrap_or_else(|| entry.join("state"));
    let state = State::prepare(&imported, &directory)?;
    atomic_json(&entry.join("identity.json"), &state.info)?;
    Ok(state)
}
