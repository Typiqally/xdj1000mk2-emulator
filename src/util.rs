use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn file_hash(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("Open {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 1024 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
pub fn owned(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            !metadata.file_type().is_symlink(),
            "Path must not be a symbolic link: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
pub fn regular(path: &Path) -> Result<()> {
    owned(path)?;
    if path.exists() {
        let m = fs::metadata(path)?;
        ensure!(
            m.is_file() && m.nlink() == 1,
            "Expected an owned regular file: {}",
            path.display()
        );
    }
    Ok(())
}
pub fn read_json(path: &Path) -> Result<Value> {
    owned(path)?;
    Ok(serde_json::from_reader(File::open(path)?)?)
}
pub fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    owned(path)?;
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("Path needs a parent")?)?;
    serde_json::to_writer_pretty(file.as_file_mut(), value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub fn exclusive(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?)
}
pub fn run(command: &mut std::process::Command) -> Result<()> {
    let display = format!("{command:?}");
    let status = command
        .status()
        .with_context(|| format!("Launch {display}"))?;
    ensure!(status.success(), "Command failed ({status}): {display}");
    Ok(())
}
