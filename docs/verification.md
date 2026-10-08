# Extraction and backend verification

The new repository has its own Git history, Rust executable, build output,
state and diagnostics. No firmware-modification tools or original firmware
assets are needed from the originating repository at build time.

## Checked locally on macOS

- Rust formatting and Clippy with warnings denied.
- 38 synthetic unit/integration/CLI tests covering update integrity and physical
  bounds, exclusive state leases, imported-source ownership, QMP size/time
  bounds, held-input release, normal/error shutdown, media preservation,
  nested rekordbox exports and ANLZ paths, cache retirement/ISA/module hashes,
  preparation predicates, and independent commands.
- More than 80% instrumented Rust line coverage across tests and native build/
  launch checks. Region coverage is approximately 74%; the unexercised
  four-session preparation orchestration is the largest gap. Coverage is not
  hardware qualification. The detailed local summary is under
  `output/coverage-summary-final.txt`.
- Fresh QEMU 9.1.0 archive, SHA-256
  `816b7022a8ba7c2ac30e2e0cf973e826f6bcc8505339603212c5ede8e94d7834`:
  all native patches applied and QEMU/DSP/qemu-img compiled independently.
  The builder regression test verifies patch application below another Git
  repository and idempotence; skipped patches cannot count as applied.
- Stock 1.45 update, SHA-256
  `6fd2a5002b297b86fb8a5931d3debf1b607ff6abaf9d49cd737d3a530002dbcd`:
  imported into private state through Rust, native boot screen inspected,
  native input/QMP accepted, screenshot saved, Swift RFB received an 800×480
  framebuffer, and the owned process exited normally. Audio was off and DSP
  compilation was disabled for this boot check. Source firmware hash unchanged.
- The real Swift desktop app launched the Rust bridge with private state and
  muted CoreAudio. Metal reported an Apple M5 Pro device, rendered guest frames,
  and displayed native panel lamp states. Its live logs opened and produced
  diagnostic snapshots. The app and backend then exited normally. No track
  was loaded, so this does not check playback.
- Optimized backend executable approximately 1.5 MiB. This is a local build
  artifact, not a performance comparison against Python.

Screenshots, native logs, engine identity and private test state are kept under
ignored `output/native-smoke/` and `output/ui-smoke/`; none are part of the public
source repository. Native source file hashes are recorded in
`native-source-manifest.json`. GitHub CI is configured but has not run remotely.

## Still unverified

The complete four-session playback/preparation sequence, cancellation during
native compilation, arbitrary user collections, cross-version endurance,
control/waveform semantics, tail latency, audio continuity and physical hardware
fidelity need further native qualification. The corresponding Rust predicates
have focused tests; that does not substitute for a real
playback run. Unmodeled hardware and the inherited interactive clock-debt cap
remain explicit limits.

The last native UI path adjustment changes only optional reference-media menu
availability: it points at user-supplied `media/reference/PIONEER` and omits those
items when the folder is absent. No personal export is bundled. The final app
was rebuilt after that change and its offscreen deck check was rerun.

Python is still used by the retained QEMU build machinery and DSP JIT generator.
The standalone player host backend itself is Rust. There has been no GitHub
publication or release packaging/notarization in this task.


## Legacy removal and native migration — 2026-10-08

The firmware repository's duplicate QEMU/DSP/Swift source and Python native host
implementations were removed after a bytewise comparison against this repository.
The pending hardware, memory, timer, audio-queue, JIT and deck changes were already
present here. The newer compact C674x decoder change was explicitly ported: RS
selects three-bit data registers and shifted two-bit register pairs, while fixed
one-bit predicate destinations stay A0/A1 or B0/B1. The component regression failed
80 subcases before the port and passed afterwards. The updated native source
manifest verified all 205 files before the subsequent Swift removal. The current
manifest verifies 198 maintained files; see [Swift removal](swift-removal.md).

Explicit shell validation now forces a fresh run rather than returning a previous
compatible verdict. Its integration regression failed before the port and passed
afterwards. Cache identity now includes the C decoder source; its regression
proved that modules made with previous register semantics cannot be seeded after
a decoder change. Prior artifacts are preserved, and NOR remains unchanged.
Separate build output and an atomic executable/data selection allow
new launches to use the rebuilt engine while existing diagnostics retain their
original build. No unrelated process was stopped or player state moved.

Checks: workspace tests, formatting and clippy with warnings denied passed;
71 Rust workspace tests and 66 native component tests passed using private DSP
libraries, plus 8 optional
Swift reference tests. Evidence is in `output/migration-*-tests.log` and
`output/migration-clippy.log`. These Python test drivers and the retained DSP
compiler do not reintroduce a Python player backend. Retired Python-host/system
fixtures remain only in the local recovery snapshot, not in the active test suite.

A clean pinned QEMU 9.1.0 build produced engine SHA-256
`f9801338fb81749c3c295a84df603e05a1ca3d4f2ec5e6aaef89b3604461b617`
under `output/engine-migrated`. Stock 1.45 firmware SHA-256
`6fd2a5002b297b86fb8a5931d3debf1b607ff6abaf9d49cd737d3a530002dbcd`
booted to its native player screen in private state with interpreter execution and
audio off. Its source bytes were unchanged. `output/migration-boot-final` contains the
bridge events, screenshot and verification receipt. This is boot evidence only;
audio continuity, control semantics and physical hardware remain unqualified.

Removed source, pending edits and retired test fixtures have a verified recovery
archive under `output/legacy-recovery`, with per-file hashes and the original
Git diff. This ignored local archive is not included in a GitHub publication.
