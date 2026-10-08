//! Local firmware discovery shared by the host and desktop. Listing never boots or imports firmware.
use serde_json::{Value, json};
use std::{fs::File, io::Read, path::Path};

pub fn archived(directory: &Path) -> Vec<Value> {
    let mut bytes = Vec::new();
    let manifest = File::open(directory.join("manifest.json"))
        .and_then(|f| f.take(1_048_577).read_to_end(&mut bytes));
    if manifest.is_err() || bytes.len() > 1_048_576 {
        return Vec::new();
    }
    let Ok(manifest) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for item in manifest["versions"].as_array().into_iter().flatten() {
        let Some(version) = item["version"].as_str() else {
            continue;
        };
        let parts: Vec<_> = version.split('.').collect();
        if parts.len() != 2
            || parts
                .iter()
                .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let Ok(major) = parts[0].parse::<u32>() else {
            continue;
        };
        let Ok(minor) = parts[1].parse::<u32>() else {
            continue;
        };
        let path = directory
            .join(format!("v{}", version.replace('.', "")))
            .join("XDJ1KMK2.UPD");
        if path.is_file() {
            entries.push((
                (major, minor),
                json!({"version":version,"path":path,"sha256":item["upd_sha256"]}),
            ));
        }
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    entries.into_iter().map(|(_, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn missing_malformed_and_oversized_archives_are_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(archived(dir.path()).is_empty());
        fs::write(dir.path().join("manifest.json"), b"oops").unwrap();
        assert!(archived(dir.path()).is_empty());
        fs::write(dir.path().join("manifest.json"), vec![b' '; 1_048_577]).unwrap();
        assert!(archived(dir.path()).is_empty());
    }
    #[test]
    fn sort_numerically_and_exclude_missing_or_unsafe_versions() {
        let dir = tempfile::tempdir().unwrap();
        for version in ["110", "19", "210"] {
            fs::create_dir(dir.path().join(format!("v{version}"))).unwrap();
            fs::write(
                dir.path().join(format!("v{version}/XDJ1KMK2.UPD")),
                b"unchanged",
            )
            .unwrap();
        }
        fs::write(
            dir.path().join("manifest.json"),
            json!({"versions":[
            {"version":"1.9"},{"version":"1.10"},{"version":"2.10"},{"version":"1.45"},
            {"version":"../../bad"},{"version":".."},{"version":""},{"version":"1."},
            {"version":"999999999999999999999.1"},{"version":123}]})
            .to_string(),
        )
        .unwrap();
        let entries = archived(dir.path());
        assert_eq!(
            entries
                .iter()
                .map(|v| v["version"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["2.10", "1.10", "1.9"]
        );
        assert_eq!(
            fs::read(dir.path().join("v110/XDJ1KMK2.UPD")).unwrap(),
            b"unchanged"
        );
    }
}
