use crate::{package::Package, util::hash};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{fs::File, io::Read, path::Path};
pub const MAX_UPDATE: usize = 64 * 1024 * 1024;
pub const STOCK_SHA: &str = "6fd2a5002b297b86fb8a5931d3debf1b607ff6abaf9d49cd737d3a530002dbcd";

pub fn profile() -> Value {
    serde_json::from_slice(include_bytes!("../native/hardware_profile.json"))
        .expect("embedded hardware profile")
}
pub fn load(path: &Path) -> Result<(Package, Vec<u8>, Value)> {
    let mut raw = Vec::new();
    File::open(path)?
        .take((MAX_UPDATE + 1) as u64)
        .read_to_end(&mut raw)?;
    ensure!(raw.len() <= MAX_UPDATE, "Update exceeds 64 MiB");
    let package = Package::parse(&raw).context("Cannot validate firmware")?;
    validate(&package)?;
    let digest = hash(&raw);
    let label = String::from_utf8_lossy(&package.main.raw[..32]);
    let version = label
        .split_once("MAINVer")
        .map(|(_, tail)| {
            tail.chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect::<String>()
        })
        .unwrap_or_else(|| "Unknown".into());
    let panel_hash = hash(&package.panel.memory);
    let info = json!({"sha256":digest,"version":version,"original":digest==STOCK_SHA || known_original(&digest,&version),"panel_sha256":panel_hash,
        "panel_original":panel_hash==profile()["panel_cpu"]["stock_image_sha256"],"model":"XDJ-1000MK2"});
    Ok((package, raw, info))
}
pub fn validate(package: &Package) -> Result<()> {
    let p = profile();
    ensure!(
        package.main.memory.len() <= p["nor"]["size_bytes"].as_u64().unwrap() as usize,
        "MAIN exceeds physical NOR"
    );
    ensure!(
        package.main.entry == p["nor"]["reset_vector"].as_u64().unwrap() as u32,
        "MAIN reset vector differs"
    );
    let allowed = p["panel_cpu"]["programmable_ranges"].as_array().unwrap();
    for &(start, end) in &package.panel.ranges {
        ensure!(
            allowed
                .iter()
                .any(|r| r[0].as_u64().unwrap() <= start as u64
                    && end as u64 <= r[1].as_u64().unwrap()),
            "PANEL writes outside physical flash"
        );
    }
    Ok(())
}
pub fn lzss_decode(packed: &[u8]) -> Result<Vec<u8>> {
    let mut ring = [b' '; 4096];
    let mut write = 4078;
    let mut output = Vec::new();
    let mut cursor = 0;
    while cursor < packed.len() {
        let flags = packed[cursor];
        cursor += 1;
        ensure!(cursor < packed.len(), "LZSS flags without a token");
        for bit in 0..8 {
            if cursor == packed.len() {
                break;
            }
            if flags & (1 << bit) != 0 {
                let value = packed[cursor];
                cursor += 1;
                output.push(value);
                ring[write] = value;
                write = (write + 1) & 4095;
            } else {
                ensure!(cursor + 2 <= packed.len(), "Truncated LZSS reference");
                let offset = packed[cursor] as usize | ((packed[cursor + 1] as usize & 240) << 4);
                let length = 3 + (packed[cursor + 1] as usize & 15);
                cursor += 2;
                for n in 0..length {
                    let value = ring[(offset + n) & 4095];
                    output.push(value);
                    ring[write] = value;
                    write = (write + 1) & 4095;
                }
            }
            if output.len() > MAX_UPDATE {
                bail!("LZSS output exceeds limit");
            }
        }
    }
    Ok(output)
}
pub fn dsp_program(path: &Path) -> Result<String> {
    let (package, _, _) = load(path)?;
    let application = lzss_decode(package.main.compressed(0x40000)?)?;
    Ok(hash(
        application
            .get(0x1000..0xa8000)
            .context("DSP image too short")?,
    ))
}

fn known_original(hash: &str, version: &str) -> bool {
    let catalog: Value = serde_json::from_slice(include_bytes!("../native/firmware_catalog.json"))
        .expect("embedded firmware catalog");
    catalog["versions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["sha256"] == hash && item["version"] == version)
}
