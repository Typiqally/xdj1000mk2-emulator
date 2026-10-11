# Swift removal and legacy cleanup — 2026-10-08

The emulator no longer retains a Swift UI, screen reader, comparison tests or
comparison launcher. Validation uses `src/ocr.rs`, which calls macOS Vision from
Rust without compiling or launching a helper. The standalone `read-screen
--image PATH` command returns the same text and normalized coordinates. Vision
OCR remains macOS specific; this does not qualify Windows/Linux playback.
The Rust-only synthetic UI benchmark remains available.

Before removal, both readers processed the same native firmware screenshot:
all 37 text rows and coordinates matched (coordinate tolerance 1e-12).
Evidence is under ignored `output/swift-removal-parity/`; the temporary Swift
executable was removed. Historical UI performance measurements, screenshots and
findings remain documented in `rust-ui.md` and their original evidence folders.
The maintained native source manifest verifies 198 files after the seven Swift
files were removed. 81 Rust workspace tests, formatting, clippy with warnings denied
and 66 native component tests passed. Instrumented tests and the matching native
screenshot reached 94.74% OCR line coverage. The installed app was rebuilt and
its signature verified. The Rust-only benchmark completed 1,790 synthetic frames
and 100 command echoes; its evidence is in `output/ui-benchmark-XAH8hL`. This is software verification, not audio or
hardware qualification.

Legacy cleanup removed 16,054 regenerable artifacts, freeing 14,841,507,840
allocated bytes (about 13.8 GiB). These were obsolete NOR/panel state and USB
overlays, compiled DSP/build/bytecode caches and Swift artifacts. Active file
paths and state leases were checked before deletion. Reports, JSON metadata,
logs, screenshots, audio and RAM/DSP captures, firmware and personal media were
preserved. Current replacement state and engines and other sessions were excluded.
The ignored receipt and per-file hash ledger are
`output/legacy-cleanup-20261008.json` and `.jsonl`.

The recovery source archive was filtered to remove Swift sources and Swift test
drivers. Its current 326-file manifest is
`output/legacy-recovery/current-source-manifest.json`; the original manifest and
Git diff remain historical evidence. `swift-removal.json` records removed hashes
and the old/new archive hashes. The obsolete Swift launcher archive was deleted.
None of these local artifacts, firmware, settings or USB media are tracked.
