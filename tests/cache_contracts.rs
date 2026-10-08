mod common;
use serde_json::json;
use std::{fs, path::Path, process::Command};
use xdj_backend::{cache, state::State, util};
fn setup(root: &Path) -> (State, std::path::PathBuf) {
    let original = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("synthetic.UPD");
    fs::write(&source, common::package()).unwrap();
    let state = State::prepare(&source, &root.join("state")).unwrap();
    let target = root.join("native/qemu/hw/cdj/c6x");
    fs::create_dir_all(target.join("tools")).unwrap();
    for name in [
        "c66x_priv.h",
        "c66x_core_int.h",
        "c66x.h",
        "c66x_decode.c",
        "tools/c14_jitgen.py",
    ] {
        fs::copy(
            original.join("native/qemu/hw/cdj/c6x").join(name),
            target.join(name),
        )
        .unwrap();
    }
    let engine = root.join("engine");
    fs::write(&engine, b"engine bytes").unwrap();
    (state, engine)
}
#[test]
fn decoder_changes_cannot_seed_code_generated_with_previous_register_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let (state, engine) = setup(dir.path());
    let original = cache::identity(dir.path(), &engine, &state).unwrap();
    let path = cache::prepare(&state, &original).unwrap();
    let batch = path.join("batch1");
    fs::create_dir(&batch).unwrap();
    fs::write(batch.join("m.cache"), cache::MODULE_VERSION).unwrap();
    module(&batch.join("m.so"), "c674x");
    let flash = util::file_hash(&state.directory.join("flash.bin")).unwrap();
    let decoder = dir.path().join("native/qemu/hw/cdj/c6x/c66x_decode.c");
    fs::write(decoder, b"changed instruction register selection").unwrap();
    fs::write(&engine, b"rebuilt engine").unwrap();
    cache::seed(dir.path(), &engine, &state, &[]).unwrap();
    let current = cache::identity(dir.path(), &engine, &state).unwrap();
    cache::prepare(&state, &current).unwrap();
    assert_ne!(original["generator_sha256"], current["generator_sha256"]);
    assert!(!path.join("batch1/m.so").exists());
    assert_eq!(
        util::file_hash(&state.directory.join("flash.bin")).unwrap(),
        flash
    );
    assert!(
        fs::read_dir(&state.directory)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.path().join("batch1/m.so").exists())
    );
}
fn module(path: &Path, isa: &str) {
    let source = path.with_extension("c");
    fs::write(
        &source,
        format!("const char c66x_jit_isa[16] = \"{isa}\";\n"),
    )
    .unwrap();
    assert!(
        Command::new("cc")
            .args(["-shared", "-fPIC"])
            .arg(source)
            .arg("-o")
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn native_module_profile_and_readiness_hashes_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let (state, engine) = setup(dir.path());
    let identity = cache::identity(dir.path(), &engine, &state).unwrap();
    let cache_path = cache::prepare(&state, &identity).unwrap();
    let batch = cache_path.join("batch1");
    fs::create_dir(&batch).unwrap();
    fs::write(
        batch.join("m.cache"),
        format!("{}\n", cache::MODULE_VERSION),
    )
    .unwrap();
    module(&batch.join("m.so"), "c674x");
    assert_eq!(
        cache::module_isa(&batch.join("m.so")).as_deref(),
        Some("c674x")
    );
    let controls = json!({"profile":3,"queue_continuity":"pass","exercised":cache::CONTROL_NAMES,"first":{"dropped_frames":0,"queue_recoveries":0,"serializer_underruns":0}});
    let mut marker = json!({"schema":1,"profile":3,"status":"ready","identity":identity,"modules":cache::inventory(&cache_path).unwrap(),"coverage":[{"controls":controls},{"controls":controls}]});
    util::atomic_json(&cache_path.join("preparation.json"), &marker).unwrap();
    assert!(cache::status(&state, &identity).is_some());
    marker["coverage"][0]["controls"]["first"]["dropped_frames"] = json!(1);
    util::atomic_json(&cache_path.join("preparation.json"), &marker).unwrap();
    assert!(cache::status(&state, &identity).is_none());
    marker["status"] = json!("unqualified");
    util::atomic_json(&cache_path.join("preparation.json"), &marker).unwrap();
    assert!(cache::status(&state, &identity).is_some());
    fs::write(batch.join("m.tried"), b"changed").unwrap();
    assert!(cache::status(&state, &identity).is_none());
}
#[test]
fn compatible_modules_survive_engine_rebuild_but_readiness_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let (state, engine) = setup(dir.path());
    let identity = cache::identity(dir.path(), &engine, &state).unwrap();
    let path = cache::prepare(&state, &identity).unwrap();
    let batch = path.join("batch2");
    fs::create_dir(&batch).unwrap();
    fs::write(batch.join("m.cache"), cache::MODULE_VERSION).unwrap();
    module(&batch.join("m.so"), "c674x");
    fs::write(path.join("idle-head"), b"0xc00353f0").unwrap();
    fs::write(path.join("preparation.json"), b"unqualified marker").unwrap();
    fs::write(&engine, b"rebuilt engine").unwrap();
    cache::seed(dir.path(), &engine, &state, &[]).unwrap();
    assert_eq!(fs::read(path.join("idle-head")).unwrap(), b"0xc00353f0");
    assert!(!path.join("preparation.json").exists());
    assert_eq!(
        util::read_json(&path.join("identity.json")).unwrap(),
        cache::identity(dir.path(), &engine, &state).unwrap()
    );
}
#[test]
fn linked_or_incomplete_batches_and_compat_isa_cannot_seed() {
    let dir = tempfile::tempdir().unwrap();
    let (state, engine) = setup(dir.path());
    let id = cache::identity(dir.path(), &engine, &state).unwrap();
    let path = cache::prepare(&state, &id).unwrap();
    let batch = path.join("batch0");
    fs::create_dir(&batch).unwrap();
    assert!(cache::batches(&path).unwrap().is_empty());
    fs::write(batch.join("m.cache"), cache::MODULE_VERSION).unwrap();
    module(&batch.join("m.so"), "compat");
    fs::write(&engine, b"new engine").unwrap();
    cache::seed(dir.path(), &engine, &state, &[]).unwrap();
    assert_eq!(util::read_json(&path.join("identity.json")).unwrap(), id);
    assert!(cache::inventory(&path).is_ok());
    std::os::unix::fs::symlink(&batch, path.join("batch10")).unwrap();
    assert_eq!(cache::batches(&path).unwrap().len(), 1);
}

/// A donor library entry: another firmware's state with one compiled batch.
fn donor(
    root: &Path,
    name: &str,
    version: &str,
    identity: &serde_json::Value,
    batch: &str,
) -> State {
    let entry = root.join(name);
    fs::create_dir_all(&entry).unwrap();
    let source = entry.join("firmware.UPD");
    fs::write(&source, common::variant(version)).unwrap();
    let state = State::prepare(&source, &entry.join("state")).unwrap();
    let cache_path = state.directory.join("dsp-jit");
    let batch = cache_path.join(batch);
    fs::create_dir_all(&batch).unwrap();
    fs::write(batch.join("m.cache"), cache::MODULE_VERSION).unwrap();
    fs::write(batch.join("m.tried"), b"0xc0000000\n").unwrap();
    fs::write(batch.join("m.qtried"), b"0xc0000020 4 0xc0000000\n").unwrap();
    module(&batch.join("m.so"), "c674x");
    fs::write(cache_path.join("idle-head"), b"0xc00353f0").unwrap();
    let mut donor_identity = identity.clone();
    donor_identity["firmware_sha256"] = json!(state.firmware_sha);
    util::atomic_json(&cache_path.join("identity.json"), &donor_identity).unwrap();
    state
}
#[test]
fn module_version_is_the_core_headers() {
    let header = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("native/qemu/hw/cdj/c6x/c66x_core_int.h"),
    )
    .unwrap();
    let version = header
        .lines()
        .find_map(|line| line.strip_prefix("#define C66X_JIT_CACHE_VERSION "))
        .unwrap();
    assert_eq!(version.trim(), cache::MODULE_VERSION);
}
#[test]
fn another_programs_modules_seed_without_the_lists_of_what_it_examined() {
    let dir = tempfile::tempdir().unwrap();
    let (state, engine) = setup(dir.path());
    let identity = cache::identity(dir.path(), &engine, &state).unwrap();
    let donors = dir.path().join("donors");
    donor(&donors, "other", "1.46", &identity, "batch3");
    cache::seed(dir.path(), &engine, &state, &[donors]).unwrap();
    let seeded = state.directory.join("dsp-jit");
    assert_eq!(
        util::read_json(&seeded.join("identity.json")).unwrap(),
        identity
    );
    assert_eq!(cache::batches(&seeded).unwrap().len(), 1);
    assert!(seeded.join("batch3/m.so").is_file());
    // Packets the donor's program could not compile may compile in this one.
    assert!(!seeded.join("batch3/m.tried").exists());
    assert!(!seeded.join("batch3/m.qtried").exists());
    assert_eq!(fs::read(seeded.join("idle-head")).unwrap(), b"0xc00353f0");
    assert!(!seeded.join("preparation.json").exists());
}
#[test]
fn a_firmwares_own_modules_outrank_a_larger_foreign_cache() {
    let dir = tempfile::tempdir().unwrap();
    let (state, engine) = setup(dir.path());
    let identity = cache::identity(dir.path(), &engine, &state).unwrap();
    let path = cache::prepare(&state, &identity).unwrap();
    let batch = path.join("batch0");
    fs::create_dir(&batch).unwrap();
    fs::write(batch.join("m.cache"), cache::MODULE_VERSION).unwrap();
    fs::write(batch.join("m.tried"), b"0xc0000000\n").unwrap();
    module(&batch.join("m.so"), "c674x");
    let donors = dir.path().join("donors");
    let other = donor(&donors, "other", "1.46", &identity, "batch1");
    let second = other.directory.join("dsp-jit/batch2");
    fs::create_dir(&second).unwrap();
    fs::write(second.join("m.cache"), cache::MODULE_VERSION).unwrap();
    module(&second.join("m.so"), "c674x");
    // The engine was rebuilt with the same generator: the cache is seeded again.
    fs::write(&engine, b"rebuilt engine").unwrap();
    cache::seed(dir.path(), &engine, &state, &[donors]).unwrap();
    let names: Vec<_> = cache::batches(&path)
        .unwrap()
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["batch0"]);
    assert!(path.join("batch0/m.tried").is_file());
}
#[test]
fn an_engine_compiles_with_the_generator_built_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _) = setup(dir.path());
    let tree = dir.path().join("native/qemu/hw/cdj/c6x");
    // No copy beside the engine: the source tree and the first build's library.
    let loose = dir.path().join("engine");
    let fallback = cache::generator(dir.path(), &loose);
    assert_eq!(fallback.sources, tree);
    assert_eq!(fallback.script, tree.join("tools/c14_jitgen.py"));
    assert_eq!(
        fallback.library,
        dir.path().join("output/engine/dsp-lib/libc66x.so")
    );
    let work = dir.path().join("output/engine-next");
    let kit = work.join("dsp-lib/c6x");
    fs::create_dir_all(kit.join("tools")).unwrap();
    fs::create_dir_all(work.join("build")).unwrap();
    let engine = work.join("build/qemu-system-sh4");
    fs::write(&engine, b"engine bytes").unwrap();
    for name in cache::GENERATOR_FILES {
        fs::copy(tree.join(name), kit.join(name)).unwrap();
    }
    // Incomplete without its library: still the tree.
    assert_eq!(cache::generator(dir.path(), &engine).sources, tree);
    fs::write(work.join("dsp-lib/libc66x.so"), b"core library").unwrap();
    let pinned = cache::generator(dir.path(), &engine);
    assert_eq!(pinned.sources, kit);
    assert_eq!(pinned.script, kit.join("tools/c14_jitgen.py"));
    assert_eq!(pinned.library, work.join("dsp-lib/libc66x.so"));
    // The tree moving on does not change what this engine's modules are built from...
    let before = cache::identity(dir.path(), &engine, &state).unwrap();
    fs::write(tree.join("c66x_core_int.h"), b"a later core").unwrap();
    assert_eq!(
        cache::identity(dir.path(), &engine, &state).unwrap(),
        before
    );
    // ...while its own generator or library changing does.
    fs::write(work.join("dsp-lib/libc66x.so"), b"another decoder").unwrap();
    assert_ne!(
        cache::identity(dir.path(), &engine, &state).unwrap()["generator_sha256"],
        before["generator_sha256"]
    );
}
