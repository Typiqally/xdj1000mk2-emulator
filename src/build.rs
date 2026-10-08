use crate::{
    cli::Args,
    state::Lease,
    util::{self, atomic_json, run},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
const QEMU_SHA: &str = "816b7022a8ba7c2ac30e2e0cf973e826f6bcc8505339603212c5ede8e94d7834";
pub fn engine(args: &Args, root: &Path) -> Result<()> {
    ensure!(
        cfg!(target_os = "macos"),
        "Native player build currently requires macOS"
    );
    let work = args
        .path("out")
        .unwrap_or_else(|| root.join("output/engine"));
    util::owned(&work)?;
    fs::create_dir_all(&work)?;
    // make and configure run from other directories: a relative --out would
    // send the DSP library into the source tree and lose configure.
    let work = work.canonicalize()?;
    let _lease = Lease::acquire(&work)?;
    let vendor = root.join("native/qemu");
    let archive = work.join("qemu-9.1.0.tar.xz");
    if !archive.exists() {
        let temporary = work.join("qemu-download.tmp");
        run(Command::new("curl")
            .args(["--fail", "--location", "--output"])
            .arg(&temporary)
            .arg("https://download.qemu.org/qemu-9.1.0.tar.xz"))?;
        ensure!(
            util::file_hash(&temporary)? == QEMU_SHA,
            "QEMU archive checksum mismatch"
        );
        fs::rename(temporary, &archive)?;
    }
    ensure!(
        util::file_hash(&archive)? == QEMU_SHA,
        "QEMU archive checksum mismatch"
    );
    let source = work.join("qemu-9.1.0");
    if !source.exists() {
        run(Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&work))?;
    }
    ensure!(
        fs::read_to_string(source.join("VERSION"))?.trim() == "9.1.0",
        "Wrong QEMU source version"
    );
    let mut patches: Vec<_> = fs::read_dir(vendor.join("patches"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "patch"))
        .collect();
    patches.sort();
    for patch in patches {
        apply_patch(&source, &patch)?;
    }
    fs::copy(
        vendor.join("patches/include_exec_cdj-pcring.h"),
        source.join("include/exec/cdj-pcring.h"),
    )?;
    let venv = work.join("build-venv");
    let python = venv.join("bin/python");
    if !python.exists() {
        run(Command::new(args.value("python").unwrap_or("python3"))
            .args(["-m", "venv"])
            .arg(&venv))?;
        run(Command::new(&python).args([
            "-m",
            "pip",
            "install",
            "meson==1.5.2",
            "ninja==1.11.1.4",
            "distlib==0.3.9",
            "tomli==2.2.1",
        ]))?;
    }
    let dsp = work.join("dsp-lib");
    run(Command::new("make")
        .arg("-C")
        .arg(vendor.join("hw/cdj/c6x"))
        .arg(format!("O={}", dsp.display()))
        .arg(dsp.join("libc66x.so")))?;
    // The generator this engine is built with stays with it (cache::generator).
    let kit = dsp.join("c6x");
    fs::create_dir_all(kit.join("tools"))?;
    for name in crate::cache::GENERATOR_FILES
        .iter()
        .chain(crate::cache::GENERATOR_HEADERS.iter())
    {
        fs::copy(vendor.join("hw/cdj/c6x").join(name), kit.join(name))?;
    }
    run(Command::new("bash")
        .arg(vendor.join("scripts/build/install_machine.sh"))
        .arg(&source)
        .env("C66X_JIT_LIBDIR", &dsp))?;
    let build = work.join("build");
    fs::create_dir_all(&build)?;
    let path = std::env::join_paths(std::iter::once(venv.join("bin")).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))?;
    if !build.join("build.ninja").exists() {
        run(Command::new(source.join("configure"))
            .current_dir(&build)
            .arg(format!("--python={}", python.display()))
            .args([
                "--target-list=sh4-softmmu",
                "--enable-cocoa",
                "--enable-coreaudio",
                "--disable-werror",
                "--disable-docs",
            ])
            .env("PATH", &path))?;
    }
    let jobs = args.number(
        "jobs",
        std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4) as u64,
    )?;
    ensure!(jobs > 0, "Jobs must be positive");
    run(Command::new("ninja")
        .arg("-C")
        .arg(&build)
        .arg(format!("-j{jobs}"))
        .args(["qemu-system-sh4", "qemu-img"])
        .env("PATH", path))?;
    atomic_json(
        &work.join("identity.json"),
        &json!({"qemu_version":"9.1.0","archive_sha256":QEMU_SHA,"engine_sha256":util::file_hash(&build.join("qemu-system-sh4"))?,"source": "native/qemu"}),
    )?;
    atomic_json(
        &root.join("output/engine.json"),
        &json!({"engine":build.join("qemu-system-sh4").canonicalize()?,
            "qemu_data":source.join("pc-bios").canonicalize()?}),
    )?;
    Ok(())
}
pub fn app(args: &Args, root: &Path) -> Result<PathBuf> {
    ensure!(cfg!(target_os = "macos"), "Mac app requires macOS");
    let target = args.path("out").unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join("Applications/XDJ-1000MK2 Emulator.app")
    });
    util::owned(&target)?;
    if target.exists() {
        ensure!(
            util::read_json(&target.join("Contents/Resources/config.json"))?["root"]
                == root.to_string_lossy().as_ref(),
            "App target is not owned by this repository"
        );
    }
    let resources = target.join("Contents/Resources");
    let macos = target.join("Contents/MacOS");
    for p in [&target.join("Contents"), &resources, &macos] {
        util::owned(p)?;
        fs::create_dir_all(p)?;
    }
    let binary = macos.join("XDJEmulator");
    let temporary = macos.join("XDJEmulator.new");
    run(Command::new("cargo").current_dir(root).args([
        "build",
        "--locked",
        "--release",
        "-p",
        "xdj-ui",
        "--bin",
        "xdj-ui",
    ]))?;
    util::regular(&temporary)?;
    fs::copy(root.join("target/release/xdj-ui"), &temporary)?;
    util::regular(&binary)?;
    fs::rename(temporary, &binary)?;
    let backend = resources.join("xdj-backend");
    let executable = std::env::current_exe()?;
    if executable != backend {
        util::regular(&backend)?;
        fs::copy(executable, &backend)?;
    }
    atomic_json(
        &resources.join("config.json"),
        &json!({"root":root,"backend":backend}),
    )?;
    let plist = r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleExecutable</key><string>XDJEmulator</string><key>CFBundleIdentifier</key><string>local.xdj1000mk2.rust-emulator</string><key>CFBundleName</key><string>XDJ-1000MK2 Emulator</string><key>CFBundleDisplayName</key><string>XDJ-1000MK2 Emulator</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleVersion</key><string>1</string><key>CFBundleShortVersionString</key><string>0.1</string><key>LSMinimumSystemVersion</key><string>13.0</string><key>NSHighResolutionCapable</key><true/></dict></plist>"#;
    util::regular(&target.join("Contents/Info.plist"))?;
    fs::write(target.join("Contents/Info.plist"), plist)?;
    run(Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(&target))?;
    if args.flag("open") {
        let mut c = Command::new("open");
        c.arg("-n").arg(&target).arg("--args");
        for name in ["firmware", "view", "ui-scale"] {
            if let Some(v) = args.value(name) {
                c.arg(format!("--{name}")).arg(v);
            }
        }
        c.arg("--");
        for name in [
            "state",
            "state-root",
            "engine",
            "qemu-data",
            "python",
            "audio",
            "compile-mode",
        ] {
            if let Some(v) = args.value(name) {
                c.arg(format!("--{name}")).arg(v);
            }
        }
        for flag in ["unmute-audio", "mute-audio", "validate"] {
            if args.flag(flag) {
                c.arg(format!("--{flag}"));
            }
        }
        run(&mut c)?;
    }
    Ok(target)
}
pub fn header(profile: &Value, bytes: &[u8]) -> Result<String> {
    let mut values = Vec::new();
    let val = |name: &str, value: &Value| -> Result<(String, u64)> {
        Ok((
            name.into(),
            value.as_u64().context("Hardware value missing")?,
        ))
    };
    for (name, group, key) in [
        ("MAIN_RAM_BASE", "main_ram", "base"),
        ("MAIN_RAM_SIZE", "main_ram", "size_bytes"),
        ("NOR_SIZE", "nor", "size_bytes"),
        ("NOR_SECTORS", "nor", "sector_count"),
        ("NOR_SECTOR_SIZE", "nor", "sector_bytes"),
        ("MAIN_CORE_HZ", "main_cpu", "core_hz"),
        ("MAIN_PVR", "main_cpu", "canonical_pvr"),
        ("MAIN_PRR", "main_cpu", "canonical_prr"),
        ("MAIN_CVR", "main_cpu", "canonical_cvr"),
        ("DSP_SDRAM_BASE", "dsp_ram", "base"),
        ("DSP_SDRAM_SIZE", "dsp_ram", "size_bytes"),
        ("DSP_REFERENCE_HZ", "dsp_cpu", "reference_hz"),
        ("DSP_MAX_HZ", "dsp_cpu", "rated_max_hz"),
        ("LCD_WIDTH", "display", "width"),
        ("LCD_HEIGHT", "display", "height"),
        ("AUDIO_HZ", "audio", "stock_output_hz"),
    ] {
        values.push(val(name, &profile[group][key])?);
    }
    let ratio = &profile["main_cpu"]["peripheral_hz"];
    values.insert(
        9,
        (
            "MAIN_PERIPH_HZ".into(),
            (ratio[0].as_u64().unwrap() as f64 / ratio[1].as_u64().unwrap() as f64)
                .round_ties_even() as u64,
        ),
    );
    for (name, group, key, index) in [
        ("RWDT_CLOCK_NUM", "rwdt", "clock_hz", 0),
        ("RWDT_CLOCK_DEN", "rwdt", "clock_hz", 1),
        ("TPU_CLOCK_NUM", "main_cpu", "bus_hz", 0),
        ("TPU_CLOCK_DEN", "main_cpu", "bus_hz", 1),
    ] {
        values.push(val(name, &profile[group][key][index])?);
    }
    for region in profile["dsp_memories"].as_array().context("DSP memories")? {
        let prefix = format!("DSP_{}", region["id"].as_str().unwrap().to_uppercase());
        for key in ["base", "alias", "size_bytes"] {
            if !region[key].is_null() {
                values.push(val(
                    &format!(
                        "{prefix}_{}",
                        if key == "size_bytes" {
                            "SIZE".into()
                        } else {
                            key.to_uppercase()
                        }
                    ),
                    &region[key],
                )?);
            }
        }
    }
    for region in profile["main_onchip_memories"]
        .as_array()
        .context("Main memories")?
    {
        let prefix = format!("MAIN_{}_RAM", region["id"].as_str().unwrap().to_uppercase());
        values.push(val(&format!("{prefix}_BASE"), &region["base"])?);
        values.push(val(&format!("{prefix}_SIZE"), &region["size_bytes"])?);
    }
    let mut lines = vec![
        "/* Generated by xdj-backend hardware --write-header. */".into(),
        "/* Capacity is modeled; cycle/physical fidelity remains unverified. */".into(),
        "#ifndef XDJ_HARDWARE_PROFILE_H".into(),
        "#define XDJ_HARDWARE_PROFILE_H".into(),
        format!("#define XDJ_PROFILE_SHA256 \"{}\"", util::hash(bytes)),
    ];
    lines.extend(
        values
            .into_iter()
            .map(|(n, v)| format!("#define XDJ_{n} {v}u")),
    );
    lines.push("#endif".into());
    Ok(lines.join("\n") + "\n")
}

/// Apply only inside the supplied standalone source tree, even below another Git repository.
pub fn apply_patch(source: &Path, patch: &Path) -> Result<()> {
    let git = |check: bool, reverse: bool| {
        let mut command = Command::new("git");
        command
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env("GIT_CEILING_DIRECTORIES", source.parent().unwrap())
            .arg("-C")
            .arg(source)
            .args(["apply", "--ignore-whitespace"]);
        if check {
            command.arg("--check");
        }
        if reverse {
            command.arg("--reverse");
        }
        command.arg(patch);
        command
    };
    let check = |reverse: bool| -> Result<bool> {
        Ok(git(true, reverse)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success())
    };
    if check(false)? {
        run(&mut git(false, false))?;
    }
    ensure!(
        check(true)?,
        "Patch not applied to pinned QEMU: {}",
        patch.display()
    );
    Ok(())
}
