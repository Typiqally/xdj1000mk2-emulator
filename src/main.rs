use anyhow::Result;
use serde_json::json;
use std::{fs, path::PathBuf};
use xdj_backend::{
    build,
    cli::{self, Args},
    firmware, media, shell, state, util, validation,
};
fn main() {
    xdj_backend::install_signals();
    let result = Args::parse(std::env::args().skip(1)).and_then(|args| run(&args));
    if let Err(e) = result {
        let _ = shell::emit(
            "error",
            json!({"message":format!("{e:#}"),"recoverable":false}),
        );
        eprintln!("{e:#}");
        std::process::exit(2);
    }
}
fn run(args: &Args) -> Result<()> {
    if ["help", "--help", "-h"].contains(&args.command.as_str()) {
        println!(
            "xdj-backend <command> [options]\n\n  build-engine    Build pinned QEMU and C DSP\n  build-app       Build/install the Rust desktop app (--open)\n  shell           Desktop JSON bridge (--firmware PATH)\n  read-screen     Native screen OCR (--image PATH)\n  inspect         Validate an update (--firmware PATH)\n  prepare         Import firmware and initialize private state\n  validate        Disposable playback/cache validation\n  hardware        Inspect or regenerate hardware profile header\n  tone            Generate test USB (--out PATH --seconds 30)\n  music           Copy audio files into USB (--out PATH FILE...)\n  export          Copy a rekordbox USB folder (--out PATH FOLDER)\n\nCommon: --root PATH, --engine PATH, --qemu-data PATH, --python PATH.\nVirtual Link: --link-listen PORT or --link-connect PORT (loopback, two players), --link-mac XX:XX:XX:XX:XX:XX, --link-trace.\nNo firmware or music is distributed. Audio is muted by default."
        );
        return Ok(());
    }
    let root = args.root()?;
    match args.command.as_str() {
        "build-engine" => build::engine(args, &root)?,
        "build-app" => {
            println!("{}", build::app(args, &root)?.display());
        }
        "read-screen" => println!("{}", xdj_backend::ocr::read(&args.required("image")?)?),
        "shell" => shell::run(args, &root)?,
        "inspect" => println!(
            "{}",
            serde_json::to_string_pretty(&firmware::load(&args.required("firmware")?)?.2)?
        ),
        "prepare" => {
            let state = state::select(
                &args.required("firmware")?,
                &args
                    .path("state-root")
                    .unwrap_or_else(|| root.join("output/firmwares")),
                args.path("state").as_deref(),
            )?;
            println!("{}", json!({"state":state.directory,"firmware":state.info}));
        }
        "validate" => {
            let state = state::select(
                &args.required("firmware")?,
                &args
                    .path("state-root")
                    .unwrap_or_else(|| root.join("output/firmwares")),
                args.path("state").as_deref(),
            )?;
            let marker =
                validation::prepare(args, &root, &state, &cli::options(args, &root)?, |event| {
                    shell::emit("preparing", event)
                })?;
            println!("{}", serde_json::to_string_pretty(&marker)?);
            anyhow::ensure!(
                marker["status"] == "ready",
                "Playback check failed; see validation artifacts"
            );
        }
        "hardware" => {
            let bytes = fs::read(root.join("native/hardware_profile.json"))?;
            let profile = serde_json::from_slice(&bytes)?;
            let header = root.join("native/qemu/hw/cdj/boards/xdj1000mk2/hardware_profile.h");
            if args.flag("write-header") {
                fs::write(&header, build::header(&profile, &bytes)?)?;
            }
            if args.flag("check-header") {
                let actual = fs::read_to_string(&header)?;
                let expected = build::header(&profile, &bytes)?;
                anyhow::ensure!(
                    actual.lines().skip(1).eq(expected.lines().skip(1)),
                    "Hardware header is stale; run hardware --write-header"
                );
            }
            if !args.flag("check-header") {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"profile":profile,"profile_sha256":util::hash(&bytes),"hardware_reference":"unverified"})
                    )?
                );
            }
        }
        "tone" => media::tone_image(
            &args.required("out")?,
            args.number("seconds", 30)?.try_into()?,
        )?,
        "music" => media::music_image(
            &args
                .positional
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>(),
            &args.required("out")?,
        )?,
        "export" => {
            let folder = args
                .positional
                .first()
                .ok_or_else(|| anyhow::anyhow!("Choose a rekordbox USB folder"))?;
            println!(
                "{}",
                media::export_image(
                    &PathBuf::from(folder),
                    &args.required("out")?,
                    args.path("audio-root").as_deref()
                )?
            );
        }
        _ => anyhow::bail!("Unknown command; run xdj-backend help"),
    }
    Ok(())
}
