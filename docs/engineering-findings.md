# Emulator findings and retained evidence

Updated 2026-10-12. This is a source and diagnostic summary. Component tests,
offline replay and short native checks have different scopes; none establishes
physical hardware equivalence or long, uninterrupted playback.

## Idle windows could stop the player

The UI's backend event queue holds 256 messages. When an occluded or idle window
stopped draining it, `try_send` failed and the reader closed the backend's stdout
pipe. A subsequent backend report could then terminate the player, closing RFB
and leaving `Firmware display: failed to fill whole buffer` on the screen.
Queue saturation is a reproduced failure path; the original user's full idle
session was not captured, so its exact trigger remains unconfirmed.

Commit `18fcad0` changes this to bounded backpressure and unblocks event senders
before joining them during shutdown. The display retries failed connections at
one-second intervals, retains the last complete image, and ignores faults from
an old connection after a player transition. Regression tests cover resuming
400 queued acknowledgements, closing a saturated queue, display recovery, and
preserving the stopped-player status.

The published source passed formatting, Clippy with warnings denied and all
100 workspace tests. A separate 125-second stock 1.45 check used private state,
interpreter mode, audio off and an induced RFB disconnect. Transport recovered,
but screenshots remained black and scheduled controls bunched when the
background UI resumed. Paused-track visual behavior and long idle endurance
remain unverified. Raw logs and identities are under
`output/display-recovery-20261012/` and `output/display-recovery-publication-*.log`.

## DSP speedups need workload-specific claims

The [optimization measurements](optimization-measurements-20261008.md) distinguish
synthetic dispatch speed from whole-session improvement. DMA invalidation batching
was 203 times faster in its alias benchmark; compiled indirect dispatch was
5.05 times faster in its synthetic loop. The retained default reduced CPU time
by 2.4% in the final paired recorded replay. Loop grouping did not show a reliable
general runtime improvement and remains opt-in.

All 36 paired replays agreed exactly. Sparse display uploads reduced preparation
p95 from 174 to 4 microseconds; this is host upload work, not firmware response
latency or audio qualification. Frozen sources, modules, replay identities,
measurements and comparison binaries remain in the evidence directory linked
from that report. Its disposable Cargo build caches and publication copies were
removed; the actual comparison artifacts were retained.

## Resume clipping remains unresolved

Archived key-shift diagnostics include a failed resume window with 89,600 clipped
right-channel frames. State samples narrow the first unusually large staging
value to the interval after cycle 152097533693 and through cycle 152101728908.
The right channel reached about `3.94e32` while the left remained about `0.22`.
These samples localize a range, not the first corrupting instruction or its cause.
An earlier recording did not reproduce resume clipping and was intentionally
cancelled; it must not be counted as a pass for the later failure.

Both recordings identify the same arm64 cache context: firmware SHA-256
`c4174a14d657d72795ce5f76fb69cd3f21143a2fa4858623ae13bc6f06305664`,
engine SHA-256
`9356f77630f1ff3019d994f3ed3ad006bf08d2358bedcd1b274b319c747a42c3`,
and generator SHA-256
`518a77b7f4da2311682d79121fe2bbe99fd8664783643f1b2e6d58e542263c36`.
The retained `record-cache/identity.json` and record-file identities keep this
diagnostic result separate from the configured runtime.

Full recordings, PCM captures, memory samples, failure screenshots, logs and
identities remain local under
`output/main-cleanup-20261012/retired-worktrees/xdj-keyshift-native-20261008/output/`.
Large execution recordings now use lossless `.bin.zst` storage. Their decompressed
SHA-256 and byte count were verified before removing the raw duplicate. Restore
a recording to a new disposable path with `zstd -d -c RECORD.bin.zst > COPY.bin`.

## Link and runtime identity boundaries

[Stock Link checks](link.md) establish discovery/status exchange and cable
recovery with two private states, interpreter execution and audio off. Separate
custom-firmware waveform checks cover analyzed CC0 tracks, zoom, master handover
and cache recovery. These do not establish stock remote waveform retrieval,
physical networking, playing-track phase accuracy or audio continuity.

The configured runtime remains `output/engine-v2/`, engine SHA-256
`4dca44a7590fd46534f7373a977a2361a0b5c6f3adf2d377776da23617c22d83`.
It predates the latest published native source. Git checkout changes do not
rebuild or requalify it. The isolated Link engine, the selected engine's generator
kit, player state, donor modules and configured three-track CC0 export are retained.
Readiness evidence belongs to the exact engine/compiler/firmware/module identity;
it must not be transferred to a rebuilt engine.

## Output retention after cleanup

The primary repository is the active checkout on `main`. Redundant publication
checkouts and retired worktree source copies were removed. The verified
`output/main-cleanup-20261012/local-history.bundle` preserves unpublished local
source and previous branch histories; it was not uploaded.

Keep `output/engine.json`, `output/desktop-defaults.json`, the selected engine,
`output/firmwares/`, `output/dsp-donors/`, `output/demo-cc0-v5/`, raw diagnostic
evidence and the small legacy recovery archive. Firmware, media, settings and
captures remain ignored local inputs. Old unused engine trees are retained as
integrity-checked `.tar.zst` archives, rather than expanded source/build copies.
Extract a historical engine into a disposable directory if it is needed again.

`output/cleanup-20261012/plan.json`, `receipts.json` and `result.json` record each
removal or lossless replacement, checksums, retained paths and measured size
reduction. Build caches are reproducible; failing evidence and identity records
are retained. Cleanup does not create a new playback, hardware or cache verdict.

The cleanup reduced allocated `output/` size from 45.1 to 20.7 GiB, a 24.5 GiB
reduction. Remaining large files include unique compressed failure recordings,
frozen optimization variants, player state and the selected runtime.
