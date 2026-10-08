//! Read-only update container validation. No host decompression or repacking.
use anyhow::{Result, anyhow};
const MAX_IMAGE: usize = 64 * 1024 * 1024;

/// Validate token framing and length without producing decoded bytes. The
/// standalone native leaf intentionally accepts some truncated streams; UPD
/// qualification retains the stricter historical host-unpack grammar.
/// Returns whether untouched dictionary bytes require a second native run with
/// the historical all-spaces initial dictionary to establish byte equivalence.
pub fn validate_lzss_tokens(packed: &[u8], expected: usize) -> Result<bool> {
    if expected > MAX_IMAGE {
        return Err(anyhow!("LZSS output exceeds limit"));
    }
    let mut initialized = [true; 4096];
    initialized[4078..].fill(false);
    let mut cursor = 0;
    let mut write = 4078;
    let mut output = 0;
    let mut tail_referenced = false;
    while cursor < packed.len() {
        let flags = packed[cursor];
        cursor += 1;
        if cursor == packed.len() {
            return Err(anyhow!("LZSS flags without a token"));
        }
        for bit in 0..8 {
            if cursor == packed.len() {
                break;
            }
            let (source, length) = if flags & (1 << bit) != 0 {
                cursor += 1;
                (None, 1)
            } else {
                if cursor + 2 > packed.len() {
                    return Err(anyhow!("Truncated LZSS reference"));
                }
                let source = packed[cursor] as usize | ((packed[cursor + 1] as usize & 240) << 4);
                let length = 3 + (packed[cursor + 1] as usize & 15);
                cursor += 2;
                (Some(source), length)
            };
            for offset in 0..length {
                if output >= MAX_IMAGE {
                    return Err(anyhow!("LZSS output exceeds limit"));
                }
                if let Some(source) = source {
                    tail_referenced |= !initialized[(source + offset) & 4095];
                }
                initialized[write] = true;
                write = (write + 1) & 4095;
                output += 1;
            }
        }
    }
    if output != expected {
        return Err(anyhow!("Packed decoded length differs"));
    }
    Ok(tail_referenced)
}

#[derive(Debug)]
pub struct Component {
    pub raw: Vec<u8>,
    pub memory: Vec<u8>,
    pub entry: u32,
    pub ranges: Vec<(usize, usize)>,
}
#[derive(Debug)]
pub struct Package {
    pub main: Component,
    pub panel: Component,
}

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for byte in bytes {
        crc ^= (*byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}
fn hex(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() % 2 != 0 {
        return Err(anyhow!("Malformed S-record hex"));
    }
    bytes
        .chunks_exact(2)
        .map(|p| {
            let digit = |b: u8| -> Result<u8> {
                match b {
                    b'0'..=b'9' => Ok(b - b'0'),
                    b'a'..=b'f' => Ok(b - b'a' + 10),
                    b'A'..=b'F' => Ok(b - b'A' + 10),
                    _ => Err(anyhow!("Malformed S-record hex")),
                }
            };
            Ok((digit(p[0])? << 4) | digit(p[1])?)
        })
        .collect()
}
impl Component {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() < 36
            || crc16(&raw[..raw.len() - 2])
                != u16::from_le_bytes(raw[raw.len() - 2..].try_into().unwrap())
        {
            return Err(anyhow!("Component CRC mismatch"));
        }
        let mut remaining = &raw[32..raw.len() - 2];
        let mut memory = Vec::new();
        let mut occupied = Vec::new();
        let mut entry = None;
        let mut ranges = Vec::new();
        while !remaining.is_empty() {
            let end = remaining
                .windows(2)
                .position(|p| p == b"\r\n")
                .unwrap_or(remaining.len());
            let line = &remaining[..end];
            remaining = if end == remaining.len() {
                &[]
            } else {
                &remaining[end + 2..]
            };
            if line.is_empty() {
                continue;
            }
            if line.len() < 3 || line[0] != b'S' || !line[1].is_ascii_digit() {
                return Err(anyhow!("Malformed S-record"));
            }
            let kind = line[1] - b'0';
            let width = match kind {
                0 | 1 | 5 | 9 => 2,
                2 | 6 | 8 => 3,
                3 | 7 => 4,
                _ => return Err(anyhow!("Unsupported S-record type")),
            };
            let bytes = hex(&line[2..])?;
            if bytes.len() < width + 2 || bytes.len() != bytes[0] as usize + 1 {
                return Err(anyhow!("S-record length mismatch"));
            }
            if bytes.iter().fold(0u8, |s, b| s.wrapping_add(*b)) != 255 {
                return Err(anyhow!("S-record checksum mismatch"));
            }
            let address = bytes[1..1 + width]
                .iter()
                .fold(0u32, |a, b| (a << 8) | *b as u32);
            let data = &bytes[1 + width..bytes.len() - 1];
            if matches!(kind, 1..=3) {
                let start = address as usize;
                let end = start
                    .checked_add(data.len())
                    .ok_or_else(|| anyhow!("S-record address exceeds image limit"))?;
                if end > MAX_IMAGE {
                    return Err(anyhow!("S-record address exceeds image limit"));
                }
                if end > memory.len() {
                    memory.resize(end, 0xff);
                    occupied.resize(end, false);
                }
                if occupied[start..end].iter().any(|b| *b) {
                    return Err(anyhow!("Overlapping S-records"));
                }
                if end > start {
                    ranges.push((start, end));
                }
                memory[start..end].copy_from_slice(data);
                occupied[start..end].fill(true);
            } else if matches!(kind, 7..=9) {
                if entry.is_some() {
                    return Err(anyhow!("Multiple entry records"));
                }
                entry = Some(address);
            }
        }
        Ok(Self {
            raw: raw.to_vec(),
            memory,
            ranges,
            entry: entry.ok_or_else(|| anyhow!("Missing entry record"))?,
        })
    }
    pub fn compressed(&self, offset: usize) -> Result<&[u8]> {
        let header = self
            .memory
            .get(offset..offset.saturating_add(4))
            .ok_or_else(|| anyhow!("Compressed block header outside image"))?;
        let length = u32::from_le_bytes(header.try_into().unwrap()) as usize;
        let end = offset
            .checked_add(4)
            .and_then(|o| o.checked_add(length))
            .ok_or_else(|| anyhow!("Compressed block extends outside image"))?;
        let check = self
            .memory
            .get(end..end.saturating_add(2))
            .ok_or_else(|| anyhow!("Compressed block extends outside image"))?;
        let sum = self.memory[offset..end]
            .iter()
            .fold(0u16, |s, b| s.wrapping_add(*b as u16));
        if sum != u16::from_le_bytes(check.try_into().unwrap()) {
            return Err(anyhow!("Compressed block checksum mismatch"));
        }
        Ok(&self.memory[offset + 4..end])
    }
}
impl Package {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let mut cursor = 0;
        let mut lengths = [0usize; 2];
        for length in &mut lengths {
            let remaining = raw
                .get(cursor..)
                .ok_or_else(|| anyhow!("Unrecognized update header"))?;
            let end = remaining
                .windows(2)
                .position(|p| p == b"\r\n")
                .ok_or_else(|| anyhow!("Unrecognized update header"))?;
            if !(1..=9).contains(&end) || !remaining[..end].iter().all(u8::is_ascii_digit) {
                return Err(anyhow!("Unrecognized update header"));
            }
            *length = std::str::from_utf8(&remaining[..end])
                .unwrap()
                .parse()
                .map_err(|_| anyhow!("Unrecognized update header"))?;
            cursor += end + 2;
        }
        if lengths.iter().any(|n| *n < 36) || cursor + lengths[0] + lengths[1] != raw.len() {
            return Err(anyhow!("Update component lengths mismatch"));
        }
        let main = Component::parse(&raw[cursor..cursor + lengths[0]])?;
        let panel = Component::parse(&raw[cursor + lengths[0]..])?;
        if !main.raw.starts_with(b"XDJ-1000MK2 MAIN") || !panel.raw.starts_with(b"XDJ-1000MK2 PANL")
        {
            return Err(anyhow!("Wrong component model or ordering"));
        }
        Ok(Self { main, panel })
    }
}
