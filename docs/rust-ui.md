# Rust desktop UI

`crates/ui` replaces the Swift host UI with egui/eframe 0.36.2 and wgpu.
The Rust UI build and launch do not use Swift. QEMU/C DSP execution is retained;
changing the UI language does not speed up guest execution or native audio.
The full engine/backend still need platform ports for Windows/Linux playback.

## Reference design

The visual reference was the former Swift deck, matching the supplied
empty firmware-view screenshot. Both faceplates preserve its coordinates and
captions. Custom painting implements the sculpted keys, selector ticks, transport
symbols, lens lamps, USB/RGB indicator, twenty jog scallops and grip dots, central
operation display, brake knob, pitch slider and calibration marks. The toolbar
and inspector retain the original hierarchy, spacing, colors and font weights.
The Keys action overlays shortcuts directly on the controls. Player view fits a
portrait window to the original hardware aspect. Resize/scale adapts the same
artwork; it does not rearrange the deck. Installed system fonts provide native
text, with egui defaults as a fallback. No system fonts are distributed.

PNG export saves the original framebuffer pixels. Pointer, held mouse/keyboard
controls, scratch/rim contact, tempo and brake values use the existing JSON panel
contract. Logs are bounded backend events and stderr. Native File/Player/View menus on macOS and in-window menus on Windows/Linux
provide recent/archived firmware, USB image and validation requests. The title context menu mirrors them. Open/Restart/Log
remain in the firmware inspector; Command/Ctrl+F opens firmware, +R restarts,
+L shows logs, +O loads music, +/ toggles keys, and +1/+2 selects the view.

## Evidence, 2026-10-08

Local evidence is under ignored `output/`; firmware, generated state, screenshots
and benchmark captures are not committed or published.

- Compared Rust screenshots with the supplied reference and the former Swift
  offscreen deck snapshot. Checked empty firmware and full player layouts,
  custom shapes, typography, toolbar icons, lamp state and inspector alignment.
- Native macOS firmware run: original 1.45 displayed its USB browser and generated
  test-tone file through QEMU/RFB in both layouts. Scripted cue and tempo inputs
  reached the backend; the screen showed +2.20% after pitch 40000. Audio was off,
  state was private, and the input firmware SHA-256 stayed unchanged. This is boot
  and UI evidence, not loaded-track playback or audio qualification.
- `cargo fmt --all --check`, workspace clippy with warnings denied, workspace
  tests, and an optimized UI build passed.
- Windows GNU x86_64 and Linux GNU x86_64 UI all-target compile checks passed.
  Those checks do not run/link a desktop app on those operating systems.
- Headless input integration tests cover holds, focus release, mouse redraws,
  keyboard input after clicking, pitch orientation and reference cap/legend
  regressions. RFB/bridge tests cover fragmented pixels, boundaries, cancellation,
  malformed messages, bounded queues and child cleanup.
- Instrumented tests plus native synthetic JSON/RFB runs reached 86.61% UI line
  coverage. Instrumented timing is excluded from performance comparisons.

The pinned QEMU executable remained
`9c7c083397dbce1801bdd82f0768a3a6339736dfdb1b1777343cb5b542f92044`.


## Performance comparison

The following comparison is historical; the Swift executable and comparison
runner have been removed. `scripts/benchmark-ui.sh` now measures only the Rust UI,
feeding local synthetic 800×480 raw full frames at 60 Hz for 30 seconds and
sending 100 timestamped echo commands.
Only each UI process is sampled for CPU and RSS; the common backend fixture is
excluded. The workload contains no firmware/DSP/audio. GPU resource reuse is
required by an automated test; same-size frames update existing textures and
bind groups rather than allocating new GPU resources every frame.

Reference-design run at commit `b2556f3` on Apple M5 Pro/Metal (Keys overlay disabled):

| Measurement | Swift | Rust |
| --- | ---: | ---: |
| Average process CPU, one core = 100% | 11.3% | 14.6% |
| Peak sampled RSS | 156.3 MiB | 190.5 MiB |
| UI executable | 0.74 MiB | 11.48 MiB |
| Peer full frames delivered | 1,800 | 1,802 |
| Synthetic command delivery p95 | 0.059 ms | 0.186 ms |
| Echo observed by GUI p95 | 0.115 ms | 16.707 ms |

The Rust GUI uploaded 1,689 frames in 30.04 seconds, about 56.2 uploads/second;
the latest-frame mailbox coalesced some peer frames. Its frame-receipt-to-upload
p95 was 8.145 ms, and upload preparation p95 was 0.180 ms. These are host-side
measurements, not physical display presentation or firmware reaction times.
Echo roundtrip includes the Rust GUI consuming queued replies on a redraw; it
is not a measurement of native panel response or audio latency.

Rust did not improve CPU, memory or executable footprint in this measured run.
Its benefit is a common UI implementation for three desktop platforms. The Swift
binary uses system UI/rendering frameworks; the Rust binary includes its UI/GPU
stack. RSS varies across runs, and this short synthetic workload is not an
endurance benchmark. Earlier simplified artwork used less memory; those values
do not describe the final reference design. The recorded Swift baseline remains in the evidence. Use the Rust benchmark
when changing the renderer or input scheduling.

The subsequent shortcut-label/placement fix does not change the default drawing
used in this comparison. The table records the measured executable, rather than
assuming that future builds have identical size or timings.

Evidence directories: `output/ui-reference-final`, `output/ui-design-check`, `output/ui-design-native-verified`,
`output/ui-final-coverage`, `output/ui-benchmark-JAOq34`. Failed exploratory runs
remain in ignored output for diagnosis and are not counted as passing evidence.

## Remaining scope

The backend still uses Unix process/state ownership, QMP Unix descriptors and a
macOS-native engine builder. CoreAudio and Vision OCR remain macOS specific.
Linux/Windows need those ports and real-machine tests before full playback can
be claimed. The desktop CI matrix is configured but has not run on GitHub.
Continuously tailed native file logs, full accessibility
qualification, portable release packaging and playback preparation parity remain follow-up work.
QEMU generation/build tools and the retained C DSP JIT generator still use Python.
The Rust host backend and Rust desktop UI do not require a Python application
server or Swift UI process.
