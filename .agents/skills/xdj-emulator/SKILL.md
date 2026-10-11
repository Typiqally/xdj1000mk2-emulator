---
name: xdj-emulator
description: Operate, diagnose and develop the standalone XDJ-1000MK2 emulator with its Rust host backend and Rust desktop UI, retaining QEMU/C DSP execution. Use for native firmware launches, virtual USB, controls, audio/DSP faults, lag, hardware fidelity and qualification. Offline firmware patch/instruction harness work stays in the firmware repository.
---

# Standalone XDJ emulator

Work from the root of `xdj1000mk2-emulator`, the supported emulator
repository. Read [AGENTS.md](../../../AGENTS.md), [README.md](../../../README.md),
[architecture](../../../docs/architecture.md) and
[verification](../../../docs/verification.md). The primary host backend and UI
are Rust. Screen OCR calls macOS Vision directly from Rust; no Swift emulator
source, comparison shell or compiler helper is retained.
Python remains in QEMU build tools and the C DSP generator.

The native player at `xdj1000mk2-custom-firmware/tools/emulator` is retired.
**Do not launch/build it, recreate its Python host, or use old binaries as a
fallback.** Its native source and Rust backend behavior were migrated before removal.
The firmware repository retains redirect stubs, offline helpers and historical
findings and evidence; obsolete legacy NOR/overlays and compiled caches were
removed after checking active paths. A verified local recovery snapshot of removed source/tests is
in `output/legacy-recovery`; it is not a supported runtime. Do not delete or
overwrite another session's work.
The firmware project's `EMULATOR.md` describes its separate offline SH4 harness.

## Commands

Check the current Rust CLI flags before running a long workload:

```sh
cd /path/to/xdj1000mk2-emulator
cargo build --locked --release
cargo build --locked --release -p xdj-ui --bin xdj-ui
./target/release/xdj-backend --help
./target/release/xdj-ui --help
# Only when an engine build is needed:
./target/release/xdj-backend build-engine --jobs 6
```

Boot-only UI check, with an absolute firmware path and a new owned state root:

```sh
./target/release/xdj-ui --firmware /absolute/path/XDJ1KMK2.UPD -- \
  --state-root output/agent-task/library --audio off --compile-mode interpreter
```

For agent automation, use `xdj-backend shell` with the same firmware/state/audio
options and `--mute-audio`; stdin/stdout is newline-delimited JSON. Existing
`src/shell.rs`, `src/controls.rs` and UI integration tests define input actions.
Use button down/up, touch, pitch, jog and release through that owned bridge.
Inspect `src/qmp.rs` and `src/runtime.rs` for owned QMP stdio and child lifecycle.
Do not open a second writer against a live player state or write UI state into
firmware RAM. The UI supports `--diagnostics` and `--test-script`; see
[desktop evidence](../../../docs/rust-ui.md). The UI compiles on Windows/Linux,
but full native engine/audio/platform playback there remains unqualified.

## Source ownership and operating invariants

- Edit maintained host code in `src/`, UI in `crates/ui/`, and QEMU/C DSP in
  `native/qemu/`. Do not edit generated copies under `output/engine/`.
  `native/hardware_profile.json` owns the generated hardware header.
- Identify firmware by absolute path and full SHA-256. Preserve its source bytes,
  user settings, USB snapshots and media. Use disposable private state and owned
  output directories; never copy test NOR/settings/overlays into a user's player.
- Keep audio monitoring muted. Boot checks use `--audio off`; those cannot qualify
  playback. Output continuity checks require muted CoreAudio and pre-mute PCM.
- Release held controls and stop only processes started by the current task.
  Preserve failed captures and startup/QEMU logs; never kill unrelated jobs.
- Keep complete firmware/engine/generator/architecture/cache identity and module
  hashes. Rebuilds need new compatible preparation and evidence; never relabel a
  cache or transfer a legacy readiness verdict to a changed engine.
- Ground hardware changes in traces, relevant manuals or physical references.
  Instruction skips, altered clocks/debt, guest patches and substitute host
  decoders cannot establish hardware fidelity.
- Keep pass, measured fail, blocked and unverified distinct. Boot/screenshots,
  transport latency, host input coverage and interpreter/compiler agreement do
  not qualify semantic controls, continuous audio or physical hardware.
- Preserve whole-session and interval accounting. A clean steady interval does
  not erase loading/control losses. Prepared readiness is not endurance acceptance.

Run the checks in AGENTS.md appropriate to a change. Report firmware/engine hashes,
workload, compile mode, intervals and evidence. Do not present older reports as
qualification of a rebuilt engine or certification of a custom image for flashing.
