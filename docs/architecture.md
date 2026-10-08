# Architecture

```text
Rust egui / wgpu / RFB
          │ newline-delimited JSON on private stdin/stdout
          ▼
Rust xdj-backend
  ├─ firmware container, CRC/S-record/physical bounds
  ├─ library imports and exclusive persistent-state leases
  ├─ immutable FAT32 media and private qcow2 overlays
  ├─ engine/compiler/firmware cache identity and module hashes
  └─ owned child lifecycle, private QMP stdio and panel input
          │ QMP + VNC Unix socket
          ▼
QEMU SH-4A / XDJ board + C674x DSP in C
          │ adaptive compilation only
          ▼
Retained Python DSP generator → system C compiler → native DSP modules
```

The native board models, DSP execution, display renderer and UI remain separate
from the Rust host backend. Rust is not assumed to make an already-native
execution engine faster. Audio and display performance depend on that engine,
its compiled DSP coverage, guest workload, timing, and host scheduling.

## Ownership

- `src/firmware.rs`, `package.rs`: update import and read-only DSP-program hash.
- `src/state.rs`: private NOR/PANEL instances, immutable imports, writer lease.
- `src/runtime.rs`, `qmp.rs`: engine command/environment, bounded synchronous
  QMP replies, owned child termination, preserved launch logs.
- `src/controls.rs`, `lamps.rs`: native inputs and observed panel-frame decoding.
- `src/media.rs`: streaming FAT32 disk creation, long filenames, rekordbox
  tree/analysis inspection, qemu-img overlays.
- `src/cache.rs`: compiler ABI, engine/firmware identities, retired caches,
  compatible donor code, module inventory and preparation-marker checks.
- `src/validation.rs`: disposable four-session checks, OCR, pre-mute PCM,
  callback accounting and guarded cache publication.
- `src/build.rs`: pinned QEMU/DSP build, hardware header and Rust desktop app packaging.
- `native/qemu`: maintained board/DSP sources and patches. Upstream QEMU is
  downloaded to ignored build output, not committed as a full source clone.
- `src/ocr.rs`: direct macOS Vision recognition, with no compiler helper.
- `native/hardware_profile.json`: sourced inventory and physical image limits.

## Compatibility

Persistent manifests retain schema 1. Cache identities retain schema 2 and
`c674x-generic-loops-v3`; generator identity covers the retained C headers, DSP script and decoder source.
The decoder was added during migration so previously decoded modules cannot be
borrowed across register-decoding changes. The C engine checks module instruction
bytes and ISA/ABI before dispatch. Preparation additionally checks module file
hashes and profile-3 input/continuity evidence. Engine hashes prevent borrowing
an old readiness verdict for a new build.

New imports use a private library by default. Importing existing persistent
state requires explicit paths. No original project's state or media is moved.
Cache retirement preserves prior artifacts and never replaces NOR or overlays.
This repository now owns native fixes. The firmware repository retains redirect
stubs and offline firmware helpers after removal of its duplicate native player.

The offline bounded SH4 harness and firmware patch builders belong to the
firmware project. They are not required by this player's host backend.

## Current limits

The backend UI contract is ported, but an end-to-end playback preparation has
not yet qualified the new repository. OCR depends on macOS Vision. CoreAudio and
Metal depend on the host desktop. Hardware equivalence, native PANEL execution,
fully modeled caches/peripherals, long playback endurance, waveform semantics,
and control latency are not established by this extraction.

The C DSP generator remains Python. Converting it would be a separate compiler
port with output/ABI/replay comparison requirements. QEMU's Meson and generation
tools also still need Python at build time. The Mac app uses repository paths;
portable signed/notarized distribution is separate work.

## Portable desktop prototype

`crates/ui` is an independent Rust UI package; it deliberately does not depend
on `xdj-backend`, which contains Unix-specific ownership and QMP code. Its
`bridge` module owns a local child and bounded JSON queues; `transport` receives
RFB through a local Unix socket or a loopback TCP address. `rfb` validates the
handshake, dimensions, encodings, rectangle bounds and message budgets before
publishing a complete frame. Only the latest frame is retained for rendering.
`app` reuses an egui texture and wakes the render thread on frames/events/input.
`input` tracks held controls and releases them on focus loss, media/firmware
transitions and shutdown. Hardware inputs still pass through the backend.

On macOS wgpu uses Metal. Windows/Linux use their available native GPU backends;
this is not a claim that the engine, audio or native packaging works there yet.
The UI source and synthetic tests can be compiled without Swift. Mac app packaging uses the Rust UI and bundles the Rust backend. Vision OCR validation calls the native framework from Rust.

The deck artwork preserves the original reference design: original faceplate
coordinates, asymmetric key caps, transport glyphs, selector, scalloped jog,
USB port/RGB indicator, tempo frame/ticks, legends, host toolbar and inspector.
`layout` owns the physical coordinates and captions; `skin` draws them without
replacing them with framework buttons. System fonts are read from the installed
OS, with bundled egui defaults as a fallback. System font files are not shipped.
The Rust app includes LCD PNG export, a keys overlay and a bounded event log.

`menus` shares File/Player/View actions between AppKit menus on macOS and the in-window menu bar on Windows/Linux. `xdj-catalog` lists local firmware for the UI and backend without starting a player. Recent-firmware history stores selected source locations separately from imported firmware/state. Ignored desktop defaults can attach an existing local USB image; explicit media arguments override them.

Continuous native file-log tailing, full accessibility
qualification, distribution packaging and playback qualification remain separate
work. The model/input protocol tests and synthetic native display tests do not
establish full historical behavior or playback parity. Swift source and comparison
tests have been removed at the user’s request; historical evidence is retained. See [desktop evidence](rust-ui.md).
