# README visuals

These are unchanged PNG captures from the real Rust/egui/wgpu emulator UI,
recorded on 2026-10-08. No AI-generated deck, screen, waveform or control is used.

| Asset | Actual view | Resolution | SHA-256 |
| --- | --- | --- | --- |
| `readme-hero.png` | Firmware view, including the LCD, panel controls and inspector | 2560 × 1596 | `f3ded54cb7fb9b1372c165ea0e52b55d5b4a2383c5d1b859d3ba4848b156a248` |
| `player-view.png` | Player view, including the LCD and deck controls | 2130 × 2820 | `39b2ffb94b1e16d3e72dde1d30a0eee9b4de5f0f12080450b3f5d03a0b82e6eb` |

The captures come from the same disposable boot session. Original XDJ-1000MK2
firmware 1.45 is browsing `TEST0001.WAV`, a synthetic test tone. The screen reads
“Not Loaded”; these images show boot and UI presentation, not playback evidence.
Audio output was disabled and host monitoring muted. No private music, personal
settings, absolute local paths, firmware files or diagnostic sidecars are bundled.

Firmware SHA-256:
`6fd2a5002b297b86fb8a5931d3debf1b607ff6abaf9d49cd737d3a530002dbcd`.

The local diagnostic scenario captured Firmware view at 20 seconds, switched to
Player view at 21 seconds and captured it at 22 seconds. The app's screenshot
handler saved the complete rendered window. The committed images are byte-for-byte
copies of those captures; README sizing only changes their display dimensions.

Original host UI material uses MIT. Firmware-rendered graphics, names and marks
remain the property of their respective owners; see [NOTICE](../../NOTICE.md).
The images do not establish hardware fidelity, continuous audio or compatibility
with a later engine build. Users must supply their own firmware to run the app.
