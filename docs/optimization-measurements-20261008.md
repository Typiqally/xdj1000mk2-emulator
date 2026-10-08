# Four optimization measurements — 2026-10-08

All four candidates were measured, implemented, and measured again in the
isolated `optimization/measure-four-20261008` worktree. The frozen starting point
is commit `ebfbca2`, including the existing uncommitted emulator source. Review
this task against that commit; the firmware repository and original emulator
working tree were not edited by this task.

The measurements below use an Apple M5 Pro, arm64, release Rust builds and
Clang `-O2` DSP builds. They establish local performance and replay agreement,
not continuous audio, cache readiness, or physical hardware qualification.

## Results

| Change | Workload | Before | After | Interpretation |
| --- | --- | ---: | ---: | --- |
| Batch packet-cache invalidation | 128 decoded blocks, two aliases, 4 KiB DMA range | 5.674 ms | 28 µs | 203× faster notification; 256 cache clears become one |
| Keep clean indirect targets compiled | B B3 loop with five delay slots | 1.2864 ns/cycle | 0.2547 ns/cycle | 5.05× faster synthetic dispatch; dynamic exits fall to zero |
| Keep fitting control-flow loops together | Complete recorded DSP session, after compilation | 18.548 CPU seconds | 18.627 CPU seconds | 0.4% slower; retain as opt-in, discovery remains the default |
| Carry display damage into GPU uploads | 80×48 updates inside an 800×480 display, 60 Hz | 174 µs upload preparation p95 | 4 µs | 97.7% less preparation; GUI CPU falls 25.61% → 23.07% of one core |

These are different workloads and must not be combined into one speedup claim.
For the recorded DSP session, invalidation batching is effectively neutral:
27.543 → 27.964 CPU seconds, with overlapping run ranges. Indirect dispatch
alone improves 18.548 → 18.207 CPU seconds (1.8%); this smaller gain is noisy.
The recorded session reports zero code generations, so code-loading DMA benefit
is exercised by the alias benchmark rather than this replay.
The display full-frame control improves preparation 179 → 31 µs (82.7%), while
GUI CPU changes only 29.09% → 28.34% (2.6%). Display input acknowledgement p95
and delivered frame counts are retained in the raw summaries. Full-frame echo
p95 medians change 6.43 → 8.49 ms, with overlapping run ranges (5.96–8.46 ms
before, 6.16–8.73 ms after); sparse echo medians change 9.26 → 5.13 ms.

The experimental combination including loop layout improves 19.344 → 18.388
CPU seconds (4.9%). Earlier individual runs during compilation suggested a
larger layout gain (25.051 → 23.161 seconds), but the quieter repeat did not
confirm it. Accordingly, loop grouping is available with `--layout loops` and
is not enabled by default. It reduces generated hop sites 5,636 → 5,227 and
eliminates four hops in the interleaved-loop fixture; those structural gains do
not establish a general runtime speedup.

The retained default combines invalidation batching and indirect dispatch,
with discovery order: **18.741 → 18.298 CPU seconds, 2.4% less CPU** in its final
paired replay comparison. Baseline samples range 18.668–19.020 seconds and
optimized samples 18.282–19.192 seconds. These modest gains remain subject to
shared-host variation; the synthetic dispatch result is not a whole-player gain.

## Implementation and correctness

- Invalidation still drops every affected decoded block and alias before
  returning. It clears the shared packet cache, advances the code generation,
  and breaks compiled execution once per notification that dropped code.
  Data-only and repeated notifications retain their fast paths.
- Indirect dispatch applies only to fully drained branches. It can enter clean
  states in the same verified function; unknown targets and pending writes
  retain materialization and interpreter fallback. Cycle-budget tests cover
  every budget from 1 to 64. A target-byte mutation through public dispatch
  matches the interpreter after invalidation.
- Iterative strongly connected components group fitting loops without exceeding
  the function state limit. Oversized components retain discovery order in
  bounded chunks. Function dependency guards and entry wrappers follow the new
  mapping. Tests cover interleaved loops, oversized loops, completed components,
  partial bins, deterministic output, the preserved default and a 3,000-state
  graph. `--layout discovery` is the measured default, `--layout loops` enables
  graph grouping, and `--no-dynamic` retains the interpreter-exit comparison.
- Display publication merges damage from overwritten mailbox frames against
  the canonical framebuffer. Initial frames and resizes remain complete.
  Uploads use the damaged rectangle and its offset; PNG export pixels retain
  the complete image. Opaque RFB pixels permit direct premultiplied conversion.
  Coalescing, small updates, resize, fragmented transport, cancellation, texture
  reuse and complete export pixels are tested.

Rust formatting, workspace Clippy with warnings denied, and workspace tests
pass: 89 passed, one previously ignored test. All 75 native tests pass. Four
baseline failures came from stale test calls to removed generator APIs; the
fixtures now exercise the current ABI and real memory callbacks. No floating
point implementation was changed for that repair.

Scoped line coverage is 98.9% of added executable generator lines, 98.3% of added
executable display lines, and 93.8% of the extracted invalidation fixture. The
layout grouping helper has 100% executable-line coverage. These are scoped
measurements, not whole-project coverage claims.

## Method and evidence

Raw artifacts are under [`output/optimization-four`](../output/optimization-four/).
The final summary is produced by
[`scripts/summarize-optimizations.py`](../scripts/summarize-optimizations.py).
`machine-identity.json` captures compiler, architecture, source commit, core,
replay, UI and fixture hashes. Each generated variant freezes its own maintained
DSP source before compiling and records generator/header/library/input/module
hashes, exact arguments and compiler logs.
All 36 paired full-session replays are exact. For the final discovery default,
all 20 profile batches were generated again with the current generator and
`--no-cc`; every emitted C file matches the measured explicit-discovery variant
byte for byte. That equivalence and the actual module producer identity are
preserved in `default-source-equivalence/identity.json`, rather than replacing
the measured module hashes with a new identity.

The recorded workload is the original read-only
`output/dsp-perf/rec1/dsp.rec`: 33,512,461 records, 52,031,351,170 recorded cycles,
8,213,899,078 executed cycles and 43,817,452,092 recorded idle-skipped cycles.
Each replay checks 12,403 state hashes, all recorded reads/writes and final RAM.
The expected final hashes are state `549e794d06814074`, writes
`61cdb10d678362e4`, RAM `5c64e469bba999ee`. The source firmware manifest identifies
SHA-256 `690dc0c96df659a5e12fbf6733cc29c95ef2000969870efa49ee3ef086ccc1ba`.
This report makes no new firmware or playback acceptance claim.

Recorded ABI 13 profiles were copied as offline compiler inputs and regenerated
against the frozen current ABI 15 core. Instruction bytes/counts were retained;
RAM inputs were read-only symlinks. This does not convert historical cache
readiness or import historical player state. No module ABI/layout was changed.

Microbenchmarks use process CPU time, 15 samples per invocation, three invocations
per version in before/after, after/before, before/after order. Warming and decoding
are outside the invalidation timer. Recorded replays run serially with order
reversal; three samples per variant are summarized by their median. Host load
is recorded because other work was active; wall-time differences are not treated
as speedups. The compiled loop benchmark checks interpreter agreement before
timing.

Display comparisons use the same frozen synthetic RFB fixture and three owned
15-second native GUI runs per version per workload in reversed order. Each run
requires at least 55 delivered peer frames/second, 100 acknowledged echo commands,
a released cue button and normal owned-process shutdown. Firmware and audio are
absent from this workload. The final UI binary is hashed separately from the
initial dirty-rectangle implementation; results above use only the final binary.

Reproduce with the frozen inputs/binaries under the evidence directory:

```sh
python3 scripts/benchmark-jit.py generate \
  --profiles /absolute/path/to/read-only/cache-A \
  --out output/new-measurement/combined-jit \
  --library /absolute/path/to/private/libc66x.so
python3 scripts/benchmark-jit.py replay \
  --binary /absolute/path/to/private/c6xreplay \
  --modules output/new-measurement/combined-jit/modules.txt \
  --record /absolute/path/to/read-only/dsp.rec \
  --out output/new-measurement/replay --runs 3
python3 scripts/benchmark-display.py \
  --binary /absolute/path/to/release/xdj-ui \
  --fixture /absolute/path/to/frozen/xdj-ui-fixture \
  --workload small --out output/new-measurement/display
python3 scripts/summarize-optimizations.py output/optimization-four
```

Build the microbenchmarks against the matching frozen DSP header. Their arguments
are a private core library and, for indirect dispatch, the generated module.
Keep all historical profiles/state read-only and use new output directories.
