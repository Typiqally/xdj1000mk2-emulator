use crate::{
    state::{Lease, State},
    util::{self, atomic_json, owned, read_json, regular},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub const FILES: [&str; 4] = ["m.so", "m.cache", "m.tried", "m.qtried"];
/// What a module of another DSP program may bring along: its code is checked
/// against the running program byte for byte before any of it is entered, but
/// the lists of packets it examined describe the program it was built from.
pub const FOREIGN_FILES: [&str; 2] = ["m.so", "m.cache"];
/// `C66X_JIT_CACHE_VERSION` of `c66x_core_int.h`, as a batch's `m.cache` holds it.
pub const MODULE_VERSION: &str = "3";
/// The files the generator reads and compiles modules against.
pub const GENERATOR_FILES: [&str; 5] = [
    "c66x_priv.h",
    "c66x_core_int.h",
    "c66x.h",
    "c66x_decode.c",
    "tools/c14_jitgen.py",
];
/// Headers a generated module includes beyond `GENERATOR_FILES`.
pub const GENERATOR_HEADERS: [&str; 2] = ["c66x_decode.h", "c66x_fenv.h"];

/// The generator an engine compiles DSP modules with: its script, the core
/// sources it reads, and the core library it asks about each packet.
pub struct Generator {
    pub sources: PathBuf,
    pub script: PathBuf,
    pub library: PathBuf,
}
/// An engine built by `build-engine` keeps the generator it was built with
/// beside it. Modules are compiled against those headers and planned with
/// that library's decoder, so both must be the engine's own: the source tree
/// moves on while an engine built from an earlier state of it keeps running.
/// An engine without that copy falls back to the tree and the first build's
/// library, as before the copy existed.
pub fn generator(root: &Path, engine: &Path) -> Generator {
    let kit = engine
        .parent()
        .and_then(Path::parent)
        .map(|work| work.join("dsp-lib"));
    if let Some(kit) = kit {
        let sources = kit.join("c6x");
        if GENERATOR_FILES.iter().all(|f| sources.join(f).is_file())
            && kit.join("libc66x.so").is_file()
        {
            return Generator {
                script: sources.join("tools/c14_jitgen.py"),
                library: kit.join("libc66x.so"),
                sources,
            };
        }
    }
    let sources = root.join("native/qemu/hw/cdj/c6x");
    Generator {
        script: sources.join("tools/c14_jitgen.py"),
        library: root.join("output/engine/dsp-lib/libc66x.so"),
        sources,
    }
}
pub fn identity(root: &Path, engine: &Path, state: &State) -> Result<Value> {
    let generator = generator(root, engine);
    let mut digest = Sha256::new();
    for name in GENERATOR_FILES {
        digest.update(fs::read(generator.sources.join(name))?);
    }
    // The decoder the generator plans with is the library's, not the source's.
    if generator.library.is_file() {
        digest.update(util::file_hash(&generator.library)?);
    }
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        a => a,
    };
    Ok(
        json!({"schema":2,"firmware_sha256":state.firmware_sha,"architecture":arch,"generator_sha256":format!("{:x}",digest.finalize()),"engine_sha256":util::file_hash(engine)?,"policy":"c674x-generic-loops-v3"}),
    )
}
pub fn prepare(state: &State, identity: &Value) -> Result<PathBuf> {
    let cache = state.directory.join("dsp-jit");
    owned(&cache)?;
    owned(&cache.join("identity.json"))?;
    let compatible = !cache.exists()
        || fs::read_dir(&cache)?.next().is_none()
        || read_json(&cache.join("identity.json")).ok().as_ref() == Some(identity);
    if !compatible {
        let retired = unique_directory(&state.directory, "dsp-jit-retired-")?;
        fs::remove_dir(&retired)?;
        fs::rename(&cache, retired)?;
    }
    fs::create_dir_all(&cache)?;
    atomic_json(&cache.join("identity.json"), identity)?;
    Ok(cache)
}
pub fn unique_directory(parent: &Path, prefix: &str) -> Result<PathBuf> {
    Ok(tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(parent)?
        .keep())
}
pub fn batches(cache: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    if !cache.exists() {
        return Ok(result);
    }
    for entry in fs::read_dir(cache)? {
        let path = entry?.path();
        let name = path.file_name().unwrap().to_string_lossy();
        if name
            .strip_prefix("batch")
            .and_then(|s| s.parse::<u64>().ok())
            .is_none()
            || owned(&path).is_err()
        {
            continue;
        }
        if regular(&path.join("m.so")).is_ok()
            && path.join("m.so").is_file()
            && owned(&path.join("m.cache")).is_ok()
            && fs::read_to_string(path.join("m.cache"))
                .ok()
                .is_some_and(|s| s.trim() == MODULE_VERSION)
        {
            result.push(path);
        }
    }
    result.sort_by_key(|p| {
        p.file_name().unwrap().to_string_lossy()[5..]
            .parse::<u64>()
            .unwrap()
    });
    Ok(result)
}
pub fn inventory(cache: &Path) -> Result<Value> {
    let mut result = serde_json::Map::new();
    for batch in batches(cache)? {
        for name in FILES {
            let p = batch.join(name);
            regular(&p)?;
            if p.exists() {
                result.insert(
                    p.strip_prefix(cache)?.to_string_lossy().into_owned(),
                    json!(util::file_hash(&p)?),
                );
            }
        }
    }
    ensure!(
        result.keys().any(|name| name.ends_with("/m.so")),
        "No complete DSP batches"
    );
    Ok(Value::Object(result))
}
pub const CONTROL_NAMES: [&str; 12] = [
    "perform",
    "play",
    "cue",
    "loop-in",
    "loop-out",
    "reloop",
    "pitch",
    "jog",
    "reverse",
    "master-tempo",
    "shortcut",
    "back",
];
pub fn status(state: &State, identity: &Value) -> Option<Value> {
    let cache = state.directory.join("dsp-jit");
    let data = read_json(&cache.join("preparation.json")).ok()?;
    if data["schema"] != 1
        || data["profile"] != 3
        || data["identity"] != *identity
        || data["modules"] != inventory(&cache).ok()?
    {
        return None;
    }
    match data["status"].as_str()? {
        "ready" => {
            let coverage = data["coverage"].as_array()?;
            if coverage.len() != 2
                || !coverage.iter().all(|row| {
                    let c = &row["controls"];
                    c["profile"] == 3
                        && c["queue_continuity"] == "pass"
                        && CONTROL_NAMES.iter().all(|n| {
                            c["exercised"]
                                .as_array()
                                .is_some_and(|v| v.contains(&json!(n)))
                        })
                        && ["dropped_frames", "queue_recoveries", "serializer_underruns"]
                            .iter()
                            .all(|k| c["first"][k] == 0)
                })
            {
                return None;
            }
        }
        "unqualified" => (),
        _ => return None,
    }
    Some(data)
}
pub fn seed(root: &Path, engine: &Path, state: &State, donors: &[PathBuf]) -> Result<()> {
    let expected = identity(root, engine, state)?;
    let _lease = Lease::acquire(&state.directory)?;
    let destination = state.directory.join("dsp-jit");
    owned(&destination)?;
    if read_json(&destination.join("identity.json")).ok().as_ref() == Some(&expected)
        && !batches(&destination)?.is_empty()
    {
        return Ok(());
    }
    let mut candidates = vec![(state.directory.clone(), Some(state.source.clone()))];
    for root in donors {
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.join("state").is_dir() {
                    candidates.push((p.join("state"), Some(p.join("firmware.UPD"))));
                }
            }
        }
    }
    let program = crate::firmware::dsp_program(&state.source).ok();
    // Prefer this firmware's own modules, then those of the same DSP program,
    // then any other program's: the more a donor shares, the more of it runs.
    let mut best = Vec::new();
    let mut best_rank = 0;
    let mut donor_head = None;
    for (folder, source) in candidates {
        let cache = folder.join("dsp-jit");
        if owned(&cache).is_err() {
            continue;
        }
        let Ok(compiled) = read_json(&cache.join("identity.json")) else {
            continue;
        };
        if ["schema", "architecture", "generator_sha256", "policy"]
            .iter()
            .any(|k| compiled[k] != expected[k])
        {
            continue;
        }
        let donor_sha = if folder == state.directory {
            state.firmware_sha.clone()
        } else {
            let Ok(manifest) = read_json(&folder.join("manifest.json")) else {
                continue;
            };
            let Some(hash) = manifest["source_sha256"].as_str() else {
                continue;
            };
            hash.to_owned()
        };
        let rank = if donor_sha == state.firmware_sha {
            3
        } else if program.is_some()
            && source
                .as_deref()
                .and_then(|p| crate::firmware::dsp_program(p).ok())
                == program
        {
            2
        } else {
            // Another DSP program. The engine enters a region only while every
            // fetch packet it was built from still holds the same bytes, and
            // compiles the packets that differ again, so what the two programs
            // share runs compiled from the first second.
            1
        };
        let modules: Vec<_> = batches(&cache)?
            .into_iter()
            .filter(|batch| module_isa(&batch.join("m.so")).as_deref() == Some("c674x"))
            .collect();
        // The C engine additionally validates instruction bytes and module ABI on load.
        if !modules.is_empty() && (rank, modules.len()) > (best_rank, best.len()) {
            donor_head = Some(cache.join("idle-head"));
            best = modules;
            best_rank = rank;
        }
    }
    if best.is_empty() {
        return Ok(());
    }
    let files: &[&str] = if best_rank == 1 {
        &FOREIGN_FILES
    } else {
        &FILES
    };
    let staging = tempfile::Builder::new()
        .prefix("dsp-jit-seeding-")
        .tempdir_in(&state.directory)?;
    for batch in best {
        let target = staging
            .path()
            .join(batch.file_name().context("Batch name")?);
        fs::create_dir(&target)?;
        for name in files {
            let p = batch.join(name);
            regular(&p)?;
            if p.exists() {
                fs::copy(p, target.join(name))?;
            }
        }
    }
    if let Some(head) = donor_head {
        if head.is_file() && regular(&head).is_ok() {
            fs::copy(head, staging.path().join("idle-head"))?;
        }
    }
    atomic_json(&staging.path().join("identity.json"), &expected)?;
    if destination.exists() {
        let retired = unique_directory(&state.directory, "dsp-jit-retired-")?;
        fs::remove_dir(&retired)?;
        fs::rename(&destination, retired)?;
    }
    fs::rename(staging.path(), &destination)?;
    Ok(())
}

/// Read the declared instruction profile from a trusted, owned native cache module.
pub fn module_isa(path: &Path) -> Option<String> {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    if regular(path).is_err() {
        return None;
    }
    let name = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: the NUL-terminated path names an explicitly selected owned cache library.
    let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: the symbol name is NUL-terminated and handle remains live until dlclose below.
    let pointer = unsafe { libc::dlsym(handle, c"c66x_jit_isa".as_ptr()) };
    let result = if pointer.is_null() {
        None
    } else {
        // SAFETY: the cache ABI declares c66x_jit_isa as a NUL-terminated static character array.
        unsafe { CStr::from_ptr(pointer.cast()) }
            .to_str()
            .ok()
            .map(str::to_owned)
    };
    // SAFETY: this balances the successful dlopen after copying the symbol's value.
    unsafe {
        libc::dlclose(handle);
    }
    result
}
