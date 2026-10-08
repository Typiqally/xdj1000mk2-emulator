use crate::{
    runtime::escape,
    state::State,
    util::{self, atomic_json, owned, read_json},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, Metadata},
    io::{Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
};
use unicode_normalization::UnicodeNormalization;
const SUPPORTED: [&str; 7] = ["wav", "mp3", "aif", "aiff", "aac", "m4a", "flac"];
struct Node {
    name: String,
    source: Option<PathBuf>,
    directory: bool,
    parent: usize,
    children: Vec<usize>,
    size: u64,
    metadata: Option<Metadata>,
    cluster: u32,
    clusters: u32,
    short: [u8; 11],
    long: Vec<u8>,
}
impl Node {
    fn directory(name: String, parent: usize) -> Self {
        Self {
            name,
            source: None,
            directory: true,
            parent,
            children: Vec::new(),
            size: 0,
            metadata: None,
            cluster: 0,
            clusters: 0,
            short: [b' '; 11],
            long: Vec::new(),
        }
    }
}
fn name_key(name: &str) -> String {
    name.nfc().collect::<String>().to_uppercase()
}
pub fn long_entries(name: &str, short: &[u8; 11]) -> Result<Vec<u8>> {
    let mut words: Vec<u16> = name.encode_utf16().collect();
    ensure!(
        words.len() <= 255 && !name.contains(['/', '\\', '\0']),
        "Invalid FAT32 name"
    );
    let checksum = short
        .iter()
        .fold(0u8, |sum, b| sum.rotate_right(1).wrapping_add(*b));
    words.push(0);
    let count = words.len().div_ceil(13);
    words.resize(count * 13, 0xffff);
    let mut result = Vec::new();
    for part in (1..=count).rev() {
        let mut entry = [0u8; 32];
        entry[0] = part as u8 | if part == count { 64 } else { 0 };
        entry[11] = 15;
        entry[13] = checksum;
        for (i, offset) in [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30]
            .iter()
            .enumerate()
        {
            entry[*offset..*offset + 2].copy_from_slice(&words[(part - 1) * 13 + i].to_le_bytes());
        }
        result.extend(entry);
    }
    Ok(result)
}
fn add_file(source: &Path, name: String, parent: usize, nodes: &mut Vec<Node>) -> Result<usize> {
    owned(source)?;
    let metadata = source.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= u32::MAX as u64,
        "USB entry must be a regular file smaller than 4 GiB"
    );
    let i = nodes.len();
    nodes.push(Node {
        name,
        source: Some(source.into()),
        directory: false,
        parent,
        children: vec![],
        size: metadata.len(),
        metadata: Some(metadata),
        cluster: 0,
        clusters: 0,
        short: [b' '; 11],
        long: vec![],
    });
    nodes[parent].children.push(i);
    Ok(i)
}
fn scan(source: &Path, parent: usize, nodes: &mut Vec<Node>, depth: usize) -> Result<usize> {
    ensure!(
        nodes.len() < 100000 && depth < 128,
        "USB tree exceeds file count or depth limit"
    );
    owned(source)?;
    let name = source
        .file_name()
        .context("Missing file name")?
        .to_str()
        .context("USB filename is not UTF-8")?
        .nfc()
        .collect();
    if source.is_dir() {
        let i = nodes.len();
        nodes.push(Node::directory(name, parent));
        nodes[parent].children.push(i);
        let mut paths: Vec<_> = fs::read_dir(source)?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<_>>()?;
        paths.sort();
        for path in paths {
            if ![".DS_Store", ".Spotlight-V100", ".fseventsd", ".Trashes"]
                .contains(&path.file_name().unwrap().to_string_lossy().as_ref())
            {
                scan(&path, i, nodes, depth + 1)?;
            }
        }
        Ok(i)
    } else {
        add_file(source, name, parent, nodes)
    }
}
pub fn music_image(paths: &[PathBuf], target: &Path) -> Result<()> {
    ensure!(
        !paths.is_empty() && paths.len() <= 100000,
        "Choose 1–100000 music files"
    );
    let mut nodes = vec![Node::directory(String::new(), 0)];
    let mut used = BTreeSet::new();
    for path in paths {
        let source = path.canonicalize()?;
        let ext = source
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        ensure!(
            SUPPORTED.contains(&ext.as_str()) && source.metadata()?.len() > 0,
            "Unsupported or empty music file"
        );
        let base = source
            .file_name()
            .unwrap()
            .to_str()
            .context("Filename must be UTF-8")?
            .nfc()
            .collect::<String>();
        let mut name = base.clone();
        let mut suffix = 2;
        while !used.insert(name_key(&name)) {
            name = format!(
                "{} ({suffix}).{ext}",
                Path::new(&base).file_stem().unwrap().to_string_lossy()
            );
            suffix += 1;
        }
        add_file(&source, name, 0, &mut nodes)?;
    }
    write_image(&mut nodes, target)
}
pub fn export_image(folder: &Path, target: &Path, audio_root: Option<&Path>) -> Result<Value> {
    let folder = folder.canonicalize()?;
    ensure!(
        folder.join("PIONEER/rekordbox/export.pdb").is_file()
            || folder.join("rekordbox/export.pdb").is_file(),
        "Choose a rekordbox USB root or its PIONEER directory"
    );
    let mut nodes = vec![Node::directory(String::new(), 0)];
    if folder.join("rekordbox/export.pdb").is_file() {
        let i = scan(&folder, 0, &mut nodes, 0)?;
        nodes[i].name = "PIONEER".into();
    } else {
        let mut children: Vec<_> = fs::read_dir(&folder)?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<_>>()?;
        children.sort();
        for child in children {
            if ![".DS_Store", ".Spotlight-V100", ".fseventsd", ".Trashes"]
                .contains(&child.file_name().unwrap().to_string_lossy().as_ref())
            {
                scan(&child, 0, &mut nodes, 0)?;
            }
        }
    }
    if let Some(audio) = audio_root {
        let source = if audio.file_name().is_some_and(|n| n == "Contents") {
            audio.to_owned()
        } else {
            audio.join("Contents")
        };
        ensure!(
            source.is_dir()
                && !nodes[0]
                    .children
                    .iter()
                    .any(|i| name_key(&nodes[*i].name) == "CONTENTS"),
            "Choose a matching Contents folder absent from this export"
        );
        scan(&source, 0, &mut nodes, 0)?;
    }
    let mut virtual_paths = BTreeMap::new();
    for i in 1..nodes.len() {
        if !nodes[i].directory {
            virtual_paths.insert(name_key(&virtual_path(i, &nodes)), i);
        }
    }
    let mut report = json!({"analysis_tracks":0,"missing_audio":[],"invalid_analysis":0,"files":virtual_paths.len()});
    for (name, i) in &virtual_paths {
        if name.contains("/USBANLZ/")
            && name.ends_with(".DAT")
            && name_key(&nodes[*i].name).starts_with("ANLZ")
        {
            match anlz_path(nodes[*i].source.as_deref().unwrap()) {
                Ok(path) => {
                    report["analysis_tracks"] =
                        json!(report["analysis_tracks"].as_u64().unwrap() + 1);
                    if !virtual_paths.contains_key(&name_key(&path)) {
                        report["missing_audio"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!(path));
                    }
                }
                Err(_) => {
                    report["invalid_analysis"] =
                        json!(report["invalid_analysis"].as_u64().unwrap() + 1)
                }
            }
        }
    }
    write_image(&mut nodes, target)?;
    Ok(report)
}
fn virtual_path(mut i: usize, nodes: &[Node]) -> String {
    let mut names = Vec::new();
    while i != 0 {
        names.push(nodes[i].name.as_str());
        i = nodes[i].parent;
    }
    names.reverse();
    format!("/{}", names.join("/"))
}
fn anlz_path(path: &Path) -> Result<String> {
    ensure!(path.metadata()?.len() <= 64 * 1024 * 1024, "ANLZ too large");
    let bytes = fs::read(path)?;
    let be = |offset: usize| -> Result<usize> {
        Ok(u32::from_be_bytes(
            bytes
                .get(offset..offset + 4)
                .context("Truncated ANLZ")?
                .try_into()
                .unwrap(),
        ) as usize)
    };
    ensure!(
        bytes.len() >= 28 && bytes.starts_with(b"PMAI") && be(8)? == bytes.len(),
        "Invalid ANLZ header"
    );
    let mut offset = be(4)?;
    ensure!(
        (28..=bytes.len()).contains(&offset),
        "ANLZ header outside file"
    );
    let mut track = None;
    let mut sections = 0;
    while offset < bytes.len() {
        sections += 1;
        ensure!(sections <= 8192, "ANLZ sections exceed limit");
        let header = be(offset + 4)?;
        let size = be(offset + 8)?;
        ensure!(
            header >= 12 && size >= header && size <= bytes.len() - offset,
            "Invalid ANLZ section"
        );
        if &bytes[offset..offset + 4] == b"PPTH" {
            ensure!(track.is_none(), "Duplicate analysis path");
            let payload = &bytes[offset + header..offset + size];
            ensure!(payload.len() % 2 == 0, "Invalid UTF-16 path");
            let words: Vec<_> = payload
                .chunks(2)
                .map(|b| u16::from_be_bytes(b.try_into().unwrap()))
                .collect();
            let path = String::from_utf16(&words)?
                .trim_end_matches('\0')
                .to_owned();
            ensure!(
                path.starts_with('/') && !path.split('/').any(|p| p == ".."),
                "Invalid analysis path"
            );
            track = Some(path);
        }
        offset += size;
    }
    track.context("Missing analysis path")
}
fn short_name(name: &str) -> Option<[u8; 11]> {
    let uppercase = name.to_uppercase();
    let (base, ext) = uppercase.rsplit_once('.').unwrap_or((&uppercase, ""));
    let valid = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"$%'!()_@~#&-".contains(&b))
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 || !valid(base) || !valid(ext) {
        return None;
    }
    let mut short = [b' '; 11];
    short[..base.len()].copy_from_slice(base.as_bytes());
    short[8..8 + ext.len()].copy_from_slice(ext.as_bytes());
    Some(short)
}
fn put16(bytes: &mut [u8], offset: usize, n: u16) {
    bytes[offset..offset + 2].copy_from_slice(&n.to_le_bytes());
}
fn put32(bytes: &mut [u8], offset: usize, n: u32) {
    bytes[offset..offset + 4].copy_from_slice(&n.to_le_bytes());
}
fn entry(short: &[u8; 11], cluster: u32, size: u32, directory: bool) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(short);
    e[11] = if directory { 16 } else { 32 };
    put16(&mut e, 20, (cluster >> 16) as u16);
    put16(&mut e, 26, cluster as u16);
    put32(&mut e, 28, if directory { 0 } else { size });
    e
}
fn write_image(nodes: &mut [Node], target: &Path) -> Result<()> {
    for i in 0..nodes.len() {
        if !nodes[i].directory {
            continue;
        }
        let mut used = BTreeSet::new();
        let mut names = BTreeSet::new();
        let literal: BTreeSet<_> = nodes[i]
            .children
            .iter()
            .filter_map(|j| short_name(&nodes[*j].name))
            .collect();
        for (number, j) in nodes[i].children.clone().into_iter().enumerate() {
            ensure!(
                names.insert(name_key(&nodes[j].name)),
                "FAT32 filenames collide"
            );
            let mut alias = number + 1;
            let mut short = short_name(&nodes[j].name);
            while short.is_none() {
                let ext = Path::new(&nodes[j].name)
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_uppercase()
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .take(3)
                    .collect::<String>();
                let s = format!("X{alias:07}{ext:3}");
                let candidate: [u8; 11] = s.as_bytes().try_into()?;
                if !used.contains(&candidate) && !literal.contains(&candidate) {
                    short = Some(candidate);
                }
                alias += 1;
            }
            let short = short.unwrap();
            ensure!(used.insert(short), "FAT32 aliases collide");
            nodes[j].short = short;
            nodes[j].long = long_entries(&nodes[j].name, &short)?;
        }
        nodes[i].size = nodes[i]
            .children
            .iter()
            .map(|j| nodes[*j].long.len() as u64 + 32)
            .sum::<u64>()
            + if i == 0 { 32 } else { 96 };
    }
    let mut cursor = 2u32;
    for n in nodes.iter_mut() {
        n.clusters = n.size.div_ceil(512) as u32;
        if n.directory {
            n.clusters = n.clusters.max(1);
        }
        n.cluster = if n.clusters > 0 { cursor } else { 0 };
        cursor = cursor
            .checked_add(n.clusters)
            .context("USB exceeds disk capacity")?;
    }
    let first = 2048u32;
    let reserved = 32u32;
    let mut sectors = 131072u32;
    let (fat_sectors, capacity) = loop {
        let fat = (sectors - reserved + 131) / 130;
        let capacity = sectors - reserved - 2 * fat;
        if cursor - 2 <= capacity {
            break (fat, capacity);
        }
        sectors = sectors
            .checked_mul(2)
            .context("USB exceeds disk capacity")?;
        ensure!(sectors <= u32::MAX - first, "USB exceeds disk capacity");
    };
    let mut boot = [0u8; 512];
    boot[..11].copy_from_slice(b"\xeb\x58\x90XDJMUSIC");
    put16(&mut boot, 11, 512);
    boot[13] = 1;
    put16(&mut boot, 14, reserved as u16);
    boot[16] = 2;
    boot[21] = 0xf8;
    put16(&mut boot, 24, 63);
    put16(&mut boot, 26, 255);
    put32(&mut boot, 28, first);
    put32(&mut boot, 32, sectors);
    put32(&mut boot, 36, fat_sectors);
    put32(&mut boot, 44, 2);
    put16(&mut boot, 48, 1);
    put16(&mut boot, 50, 6);
    boot[64] = 0x80;
    boot[66] = 0x29;
    put32(&mut boot, 67, 0x58444a33);
    boot[71..82].copy_from_slice(b"XDJ USB    ");
    boot[82..90].copy_from_slice(b"FAT32   ");
    boot[510..].copy_from_slice(b"\x55\xaa");
    let mut info = [0u8; 512];
    put32(&mut info, 0, 0x41615252);
    put32(&mut info, 484, 0x61417272);
    put32(&mut info, 488, capacity - (cursor - 2));
    put32(&mut info, 492, cursor);
    info[510..].copy_from_slice(b"\x55\xaa");
    let mut fat = vec![0u8; fat_sectors as usize * 512];
    put32(&mut fat, 0, 0x0ffffff8);
    put32(&mut fat, 4, 0xffffffff);
    for n in nodes.iter() {
        for cluster in n.cluster..n.cluster + n.clusters {
            put32(
                &mut fat,
                cluster as usize * 4,
                if cluster == n.cluster + n.clusters - 1 {
                    0x0fffffff
                } else {
                    cluster + 1
                },
            );
        }
    }
    let mut mbr = [0u8; 512];
    mbr[447..450].copy_from_slice(b"\0\x02\0");
    mbr[450] = 0x0c;
    mbr[451..454].copy_from_slice(b"\xfe\xff\xff");
    put32(&mut mbr, 454, first);
    put32(&mut mbr, 458, sectors);
    mbr[510..].copy_from_slice(b"\x55\xaa");
    let mut stream = util::exclusive(target)?;
    let result = (|| -> Result<()> {
        stream.set_len(u64::from(first + sectors) * 512)?;
        stream.write_all(&mbr)?;
        stream.seek(SeekFrom::Start(u64::from(first) * 512))?;
        stream.write_all(&boot)?;
        stream.write_all(&info)?;
        stream.seek(SeekFrom::Start(u64::from(first + 6) * 512))?;
        stream.write_all(&boot)?;
        stream.write_all(&info)?;
        stream.seek(SeekFrom::Start(u64::from(first + reserved) * 512))?;
        stream.write_all(&fat)?;
        stream.write_all(&fat)?;
        let data = u64::from(first + reserved + 2 * fat_sectors) * 512;
        for (i, node) in nodes.iter().enumerate() {
            if node.clusters == 0 {
                continue;
            }
            stream.seek(SeekFrom::Start(data + u64::from(node.cluster - 2) * 512))?;
            if node.directory {
                let mut bytes = Vec::new();
                if i != 0 {
                    bytes.extend(entry(b".          ", node.cluster, 0, true));
                    bytes.extend(entry(
                        b"..         ",
                        if node.parent == 0 {
                            0
                        } else {
                            nodes[node.parent].cluster
                        },
                        0,
                        true,
                    ));
                }
                for j in &node.children {
                    let c = &nodes[*j];
                    bytes.extend(&c.long);
                    bytes.extend(entry(&c.short, c.cluster, c.size as u32, c.directory));
                }
                bytes.resize(node.clusters as usize * 512, 0);
                stream.write_all(&bytes)?;
            } else {
                let source = node.source.as_deref().unwrap();
                let before = node.metadata.as_ref().unwrap();
                let copied = std::io::copy(&mut File::open(source)?, &mut stream)?;
                let after = source.metadata()?;
                ensure!(
                    copied == node.size
                        && (
                            before.dev(),
                            before.ino(),
                            before.mtime(),
                            before.mtime_nsec()
                        ) == (after.dev(), after.ino(), after.mtime(), after.mtime_nsec()),
                    "USB source changed while importing"
                );
            }
        }
        stream.sync_all()?;
        Ok(())
    })();
    drop(stream);
    if result.is_err() {
        let _ = fs::remove_file(target);
    }
    result
}
pub fn tone_image(target: &Path, seconds: u32) -> Result<()> {
    ensure!(
        (1..=900).contains(&seconds),
        "Tone duration must be 1–900 seconds"
    );
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("TEST0001.WAV");
    let mut file = File::create(&path)?;
    let length = seconds * 44100 * 4;
    let mut header = [0u8; 44];
    header[..4].copy_from_slice(b"RIFF");
    put32(&mut header, 4, length + 36);
    header[8..16].copy_from_slice(b"WAVEfmt ");
    put32(&mut header, 16, 16);
    put16(&mut header, 20, 1);
    put16(&mut header, 22, 2);
    put32(&mut header, 24, 44100);
    put32(&mut header, 28, 176400);
    put16(&mut header, 32, 4);
    put16(&mut header, 34, 16);
    header[36..40].copy_from_slice(b"data");
    put32(&mut header, 40, length);
    file.write_all(&header)?;
    let mut buffer = Vec::with_capacity(44100 * 4);
    for s in 0..seconds {
        buffer.clear();
        for sample in s * 44100..(s + 1) * 44100 {
            for hz in [440., 880.] {
                let v = (6000.
                    * (2. * std::f64::consts::PI * hz * f64::from(sample) / 44100.).sin())
                .round_ties_even() as i16;
                buffer.extend(v.to_le_bytes());
            }
        }
        file.write_all(&buffer)?;
    }
    drop(file);
    music_image(&[path], target)
}
pub fn overlay(source: &Path, state: &State, qemu_img: &Path) -> Result<PathBuf> {
    let source = source.canonicalize()?;
    ensure!(
        source.is_file() && source.parent() != Some(state.directory.as_path()),
        "USB backing image must be outside player state"
    );
    let digest = util::file_hash(&source)?;
    let directory = state.directory.join("collections").join(&digest);
    owned(&state.directory.join("collections"))?;
    owned(&directory)?;
    fs::create_dir_all(&directory)?;
    let overlay = directory.join("usb.qcow2");
    let manifest = directory.join("usb.json");
    for p in [&overlay, &manifest] {
        util::regular(p)?;
    }
    let identity =
        json!({"schema":1,"source":source,"sha256":digest,"size":source.metadata()?.len()});
    if overlay.exists() || manifest.exists() {
        ensure!(
            overlay.is_file() && read_json(&manifest)? == identity,
            "USB backing image changed or state incomplete"
        );
    } else {
        let result = Command::new(qemu_img)
            .args(["create", "-q", "-f", "qcow2", "-F", "raw", "-b"])
            .arg(&source)
            .arg(&overlay)
            .output()?;
        ensure!(
            result.status.success(),
            "qemu-img: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        atomic_json(&manifest, &identity)?;
    }
    Ok(overlay)
}
pub fn options(overlay: &Path) -> Vec<String> {
    vec![
        "-drive".into(),
        format!("if=none,id=xdj-usb,format=qcow2,file={}", escape(overlay)),
        "-device".into(),
        "xdj-usb-flash,drive=xdj-usb,port=1,removable=on".into(),
    ]
}
