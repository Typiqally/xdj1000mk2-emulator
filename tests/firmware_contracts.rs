mod common;
use std::fs;
use xdj_backend::{
    build, firmware,
    package::Package,
    state::{State, select},
    util,
};
#[test]
fn valid_package_builds_private_nor_and_preserves_input() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("synthetic.UPD");
    let bytes = common::package();
    fs::write(&source, &bytes).unwrap();
    let (_, _, info) = firmware::load(&source).unwrap();
    assert_eq!(info["version"], "1.45");
    assert_eq!(info["original"], false);
    let state = select(&source, &dir.path().join("library"), None).unwrap();
    assert!(state.source.metadata().unwrap().permissions().readonly());
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(
        state.directory.join("flash.bin").metadata().unwrap().len(),
        16777216
    );
    let again = State::prepare(&state.source, &state.directory).unwrap();
    assert_eq!(again.firmware_sha, state.firmware_sha);
    fs::write(state.directory.join("panel.bin"), b"altered").unwrap();
    assert!(State::prepare(&state.source, &state.directory).is_err());
}
#[test]
fn crc_record_and_component_lengths_fail_closed() {
    let bytes = common::package();
    for position in [0, 20, 55, 100, bytes.len() - 1] {
        let mut changed = bytes.clone();
        changed[position] ^= 128;
        assert!(Package::parse(&changed).is_err());
    }
    for len in [0, 1, 15, 32, bytes.len() - 1] {
        assert!(Package::parse(&bytes[..len]).is_err());
    }
}
#[test]
fn imports_refuse_linked_library_and_same_source_directory() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("synthetic.UPD");
    fs::write(&source, common::package()).unwrap();
    std::os::unix::fs::symlink(dir.path(), dir.path().join("linked")).unwrap();
    assert!(select(&source, &dir.path().join("linked"), None).is_err());
    assert!(select(&source, &dir.path().join("library"), Some(dir.path())).is_err());
}
#[test]
fn hardware_header_has_exact_existing_definitions() {
    let profile = firmware::profile();
    let bytes = include_bytes!("../native/hardware_profile.json");
    let generated = build::header(&profile, bytes).unwrap();
    let existing = include_str!("../native/qemu/hw/cdj/boards/xdj1000mk2/hardware_profile.h");
    assert_eq!(
        generated.lines().skip(1).collect::<Vec<_>>(),
        existing.lines().skip(1).collect::<Vec<_>>()
    );
    assert_eq!(util::hash(bytes), profile_hash(&generated));
}
fn profile_hash(header: &str) -> String {
    header
        .lines()
        .find(|line| line.starts_with("#define XDJ_PROFILE_SHA256"))
        .unwrap()
        .split('"')
        .nth(1)
        .unwrap()
        .into()
}
#[test]
fn corrupted_packed_block_is_rejected_without_panicking() {
    let package = Package::parse(&common::package()).unwrap();
    assert!(package.main.compressed(usize::MAX).is_err());
    assert!(package.main.compressed(0).is_err());
}
#[test]
fn engine_patches_apply_inside_nested_source_tree_and_are_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(dir.path())
            .status()
            .unwrap()
            .success()
    );
    let source = dir.path().join("output/engine/source");
    fs::create_dir_all(&source).unwrap();
    let file = source.join("fixture.txt");
    fs::write(&file, "old\n").unwrap();
    let patch = dir.path().join("change.patch");
    fs::write(
        &patch,
        "--- a/fixture.txt\n+++ b/fixture.txt\n@@ -1 +1 @@\n-old\n+new\n",
    )
    .unwrap();
    build::apply_patch(&source, &patch).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "new\n");
    build::apply_patch(&source, &patch).unwrap();
}
#[test]
fn component_records_reject_overlap_and_duplicate_entries() {
    let base = common::component("XDJ-1000MK2 MAINVer1.45", 0, 0xa0000000);
    for record in [
        common::record(3, 0, &[9]),
        common::record(7, 0xa0000000, &[]),
    ] {
        let mut bytes = base[..base.len() - 2].to_vec();
        bytes.extend(record.bytes());
        bytes.extend(common::crc16(&bytes).to_le_bytes());
        assert!(xdj_backend::package::Component::parse(&bytes).is_err());
    }
}
#[test]
fn compressed_block_bounds_sum_and_token_validation_are_exact() {
    use xdj_backend::package::{Component, validate_lzss_tokens};
    let packed = [1, b'A'];
    let mut memory = (packed.len() as u32).to_le_bytes().to_vec();
    memory.extend(packed);
    let sum = memory.iter().map(|b| u16::from(*b)).sum::<u16>();
    memory.extend(sum.to_le_bytes());
    let mut component = Component {
        raw: vec![],
        memory,
        entry: 0,
        ranges: vec![],
    };
    assert_eq!(component.compressed(0).unwrap(), packed);
    component.memory[5] ^= 1;
    assert!(component.compressed(0).is_err());
    assert!(!validate_lzss_tokens(&packed, 1).unwrap());
    assert!(validate_lzss_tokens(&[0, 0], 1).is_err());
    assert!(validate_lzss_tokens(&packed, 2).is_err());
    assert!(validate_lzss_tokens(&[0], 0).is_err());
    assert!(validate_lzss_tokens(&[0, 0xee, 0xf0], 3).unwrap());
}
