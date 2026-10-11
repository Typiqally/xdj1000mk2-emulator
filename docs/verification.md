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

## DSP execution speed — 2026-10-08

Measured failure, engine `f9801338…` with a warm cache, key-shift firmware
`690dc0c9…`, private state, muted output: with MASTER TEMPO on at +3.45 % the DAC
received 12,394 frames/s over a 33 s interval, 23.6 s of DSP time was forgiven by
the 300 ms lag cap, and MAIN waited for the DSP lock 59 % of the time. A
recording of that session (`CDJ_XDJ_DSP_RECORD`, 52.03 G cycles) replayed offline
showed why: the time-stretch code needs 130 M cycles/s and the core delivered 30 M
uncompiled and 133 M compiled. The causes and changes are in
`native/qemu/hw/cdj/c6x/c66x_core.md` ("Compiled regions in the machine").

Pass:

- The recording replays `EXACT` on the changed core, interpreted and compiled:
  12,403 state hashes, 1,476,262 bus writes with no value mismatch, final RAM
  hash `0x5c64e469bba999ee`, the same as the unchanged core's.
- Offline, host seconds per DSP second: track load 0.34 → 0.23 (worst second
  1.36 → 0.80), playback 0.19 → 0.14, MASTER TEMPO 0.97 → 0.26; uncompiled
  MASTER TEMPO 4.6 → 1.7.
- 68 native component tests and 86 Rust workspace tests; formatting and clippy
  with warnings denied. The generator, its headers and its core library are now
  copied beside each engine build, and a rebuilt source tree keeps its patch set.
- In the machine, engine `4dca44a7…` (`output/engine-v2`), key-shift 0.3 firmware
  `81a60919…` in a new private state seeded from `output/dsp-donors`, muted
  CoreAudio, driven through the shell bridge: track load, 20 s of playback and
  41 s of MASTER TEMPO at 44,100 frames/s with 0 ms forgiven, no dropped frame
  and no serializer underrun for the whole 125 s session. The donor cache was
  compiled by replaying the recording with that engine's generator; the 0.3
  firmware's DSP program differs from the recorded one in about 1 % of its
  fetch packets, which the engine compiled again while it ran.

Measured fail, kept:

- A cold cache still starts interpreted. The same session without a donor
  delivered 15,000–33,000 frames/s for its first minutes, and under a host load
  average above 30 from unrelated work it never settled.
- While these changes were in the source tree, sessions launched by the
  installed backend with the engine of the moment could not compile at all (29
  failed batches in one): that backend runs the tree's generator against whatever
  engine is selected. An engine built from here on carries its own.

Unverified:

- Four-session playback preparation, audio continuity by PCM analysis, control
  semantics, long endurance and physical hardware. Nothing here qualifies a cache
  as `ready`.
- Timing of the final engine on an otherwise idle host, and of the transport
  controls beyond load, play and MASTER TEMPO.
- The installed desktop app: it was not rebuilt, so it does not yet seed from
  `output/dsp-donors` or use the engine's own generator.
- A host-port read of DSP memory no longer stops the DSP thread; that it is
  equivalent for the firmware rests on the board's own asynchronous host port,
  not on a hardware trace.
