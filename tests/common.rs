#![allow(dead_code)]
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
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
pub fn record(kind: u8, address: u32, data: &[u8]) -> String {
    let width = if kind == 3 || kind == 7 { 4 } else { 3 };
    let mut raw = vec![(width + data.len() + 1) as u8];
    raw.extend_from_slice(&address.to_be_bytes()[4 - width..]);
    raw.extend(data);
    raw.push(!raw.iter().fold(0u8, |s, b| s.wrapping_add(*b)));
    format!(
        "S{kind}{}\r\n",
        raw.iter().map(|b| format!("{b:02X}")).collect::<String>()
    )
}
pub fn component(label: &str, address: u32, entry: u32) -> Vec<u8> {
    let mut raw = vec![b' '; 32];
    raw[..label.len()].copy_from_slice(label.as_bytes());
    raw.extend(record(3, address, &[1, 2, 3, 4]).bytes());
    raw.extend(record(7, entry, &[]).bytes());
    raw.extend(crc16(&raw).to_le_bytes());
    raw
}
pub fn package() -> Vec<u8> {
    let main = component("XDJ-1000MK2 MAINVer1.45", 0, 0xa0000000);
    let panel = component("XDJ-1000MK2 PANLVer1.00", 0xc0000, 0xffffe);
    let mut raw = format!("{}\r\n{}\r\n", main.len(), panel.len()).into_bytes();
    raw.extend(main);
    raw.extend(panel);
    raw
}

pub fn variant(version: &str) -> Vec<u8> {
    let main = component(&format!("XDJ-1000MK2 MAINVer{version}"), 0, 0xa0000000);
    let panel = component("XDJ-1000MK2 PANLVer1.00", 0xc0000, 0xffffe);
    let mut bytes = format!("{}\r\n{}\r\n", main.len(), panel.len()).into_bytes();
    bytes.extend(main);
    bytes.extend(panel);
    bytes
}
