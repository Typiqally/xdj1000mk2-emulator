use serde_json::{Value, json};
pub fn decode(raw: &str) -> Value {
    let dark = json!({"lit":[],"usb":[0,0,0],"jog":{"position":null,"vinyl":false,"touch":false}});
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return dark;
    };
    let Some(hex) = v["frame"].as_str() else {
        return dark;
    };
    if hex.len() != 64 {
        return dark;
    }
    let Ok(frame) = hex
        .as_bytes()
        .chunks(2)
        .map(|s| {
            std::str::from_utf8(s)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .ok_or(())
        })
        .collect::<Result<Vec<_>, _>>()
    else {
        return dark;
    };
    let checksum = frame[..30].iter().fold(0u16, |sum, b| {
        let n = sum + u16::from(*b);
        (n & 255) + (n >> 8)
    });
    if frame[31] != 0x8f || checksum as u8 != frame[30] {
        return dark;
    }
    let mut lit = Vec::new();
    for (name, byte, mask) in [
        ("play", 0, 1),
        ("cue", 0, 2),
        ("reverse", 0, 4),
        ("loop-in", 0, 16),
        ("loop-out", 0, 32),
        ("reloop", 0, 64),
        ("vinyl", 1, 2),
        ("master-tempo", 1, 4),
        ("quantize", 1, 8),
        ("selector", 1, 16),
    ] {
        if frame[byte] & mask != 0 {
            lit.push(name);
        }
    }
    if frame[22..25].iter().any(|b| *b >= 16) {
        lit.push("usb");
    }
    lit.sort();
    let rgb: Vec<u64> = frame[22..25]
        .iter()
        .map(|b| ((f64::from(*b) * 255.0 / 40.0).round_ties_even() as u64).min(255))
        .collect();
    json!({"lit":lit,"usb":rgb,"jog":{"position":if (1..=135).contains(&frame[9]){Some(frame[9])}else{None},"vinyl":frame[8]&8!=0,"touch":frame[8]&32!=0}})
}
