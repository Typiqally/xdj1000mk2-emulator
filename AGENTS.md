# Emulator agent entry point

For native emulator, controls, audio/DSP, hardware, performance or UI work, read
[.agents/skills/xdj-emulator/SKILL.md](.agents/skills/xdj-emulator/SKILL.md).
This is the supported replacement for the firmware repository's retired native
Python/Swift player. New emulator work belongs here; the firmware project's offline
SH4/package harness remains there. The legacy native source has been migrated and removed from that repository.
Historical qualification stays tied to its original engine.

# Native emulator backend

This repository contains a Rust host backend, retained QEMU/C DSP sources,
a Rust desktop UI, and Rust/Vision screen recognition. Work on maintained native sources under `native/`,
never generated source under `output/`.

Run `cargo fmt --all --check`, `cargo clippy --locked --workspace --all-targets -- -D warnings`
and `cargo test --locked --workspace` after Rust changes. Native changes also run
`python3 -m unittest discover -s tests/native -v` (private C/DSP builds; no firmware
required).
Firmware, media and player settings
are local inputs; never commit them or copy factory test state over user state.
Use private disposable state for native diagnostics, muted CoreAudio for output
checks, and audio-off for boot-only checks. Stop only processes started by the
current task. Preserve source hashes, logs, failing evidence and cache identity.

Boot, screenshots and host input coverage do not qualify audio or physical
hardware. Keep pass, fail, blocked and unverified distinct. Never mark a cache
ready without compiler-free playback, queue and callback evidence. The retained
DSP JIT and QEMU build tools still need Python; the host backend is Rust.

Keep responses short. Re-diagnose after two unsuccessful fixes to one issue.
Before introducing material compatibility, publication or infrastructure work,
explain the commitment and honor scope already approved by the user.

# Interactive demo media

Use the three-track CC0 demo configured in `output/desktop-defaults.json` for
interactive launches, including linked players. It contains precomputed waveforms
and beat grids; see `docs/demo-media.md` to rebuild it. Use the user's personal
library only when explicitly requested. Do not replace analyzed demo media with
`output/media/demo.img` or a generated test WAV unless explicitly requested or
needed for a scoped audio test.
Keep the source image immutable and use each player's private overlay.
Personal audio, databases, analysis, library paths and screenshots belong only
in ignored local folders. Use synthetic data in publishable examples; do not
copy user library metadata or captures into source, documentation or commits.
