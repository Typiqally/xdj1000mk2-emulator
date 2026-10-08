use std::{
    fs,
    io::{Read, Seek, SeekFrom},
};
use xdj_backend::media;
#[test]
fn media_preserves_sources_and_writes_a_real_fat32_partition() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("Track α.wav");
    fs::write(&source, b"audio unchanged").unwrap();
    let target = dir.path().join("usb.img");
    media::music_image(std::slice::from_ref(&source), &target).unwrap();
    assert_eq!(fs::read(&source).unwrap(), b"audio unchanged");
    let mut file = fs::File::open(&target).unwrap();
    let mut sector = [0u8; 512];
    file.read_exact(&mut sector).unwrap();
    assert_eq!(&sector[510..], b"\x55\xaa");
    assert_eq!(sector[450], 0x0c);
    file.seek(SeekFrom::Start(2048 * 512)).unwrap();
    file.read_exact(&mut sector).unwrap();
    assert_eq!(&sector[82..90], b"FAT32   ");
    assert_eq!(u16::from_le_bytes(sector[11..13].try_into().unwrap()), 512);
    assert!(media::music_image(&[source], &target).is_err());
}
#[test]
fn export_preserves_nested_contents_and_analysis_report() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("USB");
    fs::create_dir_all(root.join("PIONEER/rekordbox")).unwrap();
    fs::write(root.join("PIONEER/rekordbox/export.pdb"), b"pdb preserved").unwrap();
    fs::create_dir_all(root.join("Contents/Artist")).unwrap();
    fs::write(root.join("Contents/Artist/song.mp3"), b"song preserved").unwrap();
    let image = dir.path().join("export.img");
    let report = media::export_image(&root, &image, None).unwrap();
    assert_eq!(report["files"], 2);
    assert_eq!(report["analysis_tracks"], 0);
    let bytes = fs::read(image).unwrap();
    assert!(bytes.windows(13).any(|w| w == b"pdb preserved"));
    assert!(bytes.windows(14).any(|w| w == b"song preserved"));
}
#[test]
fn export_refuses_links_and_fat_name_collisions() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("PIONEER/rekordbox")).unwrap();
    fs::write(dir.path().join("PIONEER/rekordbox/export.pdb"), b"").unwrap();
    std::os::unix::fs::symlink("PIONEER", dir.path().join("link")).unwrap();
    let output = dir.path().join("bad.img");
    assert!(media::export_image(dir.path(), &output, None).is_err());
    assert!(!output.exists());
}
#[test]
fn empty_music_collection_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    assert!(media::music_image(&[], &dir.path().join("bad.img")).is_err());
}
#[test]
fn export_checks_analysis_paths_and_finds_missing_audio() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("USB");
    fs::create_dir_all(root.join("PIONEER/rekordbox")).unwrap();
    fs::write(root.join("PIONEER/rekordbox/export.pdb"), b"pdb").unwrap();
    fs::create_dir_all(root.join("PIONEER/USBANLZ/one")).unwrap();
    let path = "/Contents/missing.mp3";
    let payload: Vec<u8> = path
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_be_bytes)
        .collect();
    let mut bytes = vec![0u8; 28];
    bytes[..4].copy_from_slice(b"PMAI");
    bytes[4..8].copy_from_slice(&28u32.to_be_bytes());
    let mut section = vec![0u8; 20];
    section[..4].copy_from_slice(b"PPTH");
    section[4..8].copy_from_slice(&20u32.to_be_bytes());
    section[8..12].copy_from_slice(&((20 + payload.len()) as u32).to_be_bytes());
    section.extend(payload);
    bytes.extend(section);
    let length = bytes.len() as u32;
    bytes[8..12].copy_from_slice(&length.to_be_bytes());
    fs::write(root.join("PIONEER/USBANLZ/one/ANLZ0000.DAT"), &bytes).unwrap();
    fs::write(root.join("PIONEER/USBANLZ/one/ANLZ0001.DAT"), b"invalid").unwrap();
    let report = media::export_image(&root, &dir.path().join("usb.img"), None).unwrap();
    assert_eq!(report["analysis_tracks"], 1);
    assert_eq!(report["invalid_analysis"], 1);
    assert_eq!(report["missing_audio"], serde_json::json!([path]));
}
#[test]
fn long_filenames_are_bounded_and_duplicate_music_names_are_renamed() {
    let short = *b"TRACK001WAV";
    assert!(media::long_entries(&"x".repeat(256), &short).is_err());
    assert!(media::long_entries("path/file", &short).is_err());
    let entries = media::long_entries("long name.wav", &short).unwrap();
    assert_eq!(entries[11], 15);
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("track.wav");
    fs::write(&source, b"a").unwrap();
    let output = dir.path().join("music.img");
    media::music_image(&[source.clone(), source], &output).unwrap();
    assert!(output.is_file());
}
