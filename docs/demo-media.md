# Three-track analyzed demo

Interactive demonstrations use these public tracks by MintoDog, offered under
CC0 by the creator:

| Track | Creator tempo | Source and license declaration |
| --- | --- | --- |
| Hard Dungeon | 140 BPM | [OpenGameArt](https://opengameart.org/content/hard-dungeon) |
| Hot Roadway | 160 BPM | [OpenGameArt](https://opengameart.org/content/hot-roadway) |
| Joyfully (Loop) | 170 BPM | [OpenGameArt](https://opengameart.org/content/joyfully) |

Source URLs and pinned audio hashes are in `scripts/demo-tracks.json`. Audio,
export databases, analysis and images remain in ignored local output. No user's
audio, database rows, waveform data, artwork or metadata are used as templates.

Build with installed ffmpeg and the optimized Rust backend:

```sh
cargo build --locked --release --bin xdj-backend
python3 scripts/build_demo_media.py --out output/demo-cc0-new \
  --backend target/release/xdj-backend
```

The destination must be new. The tool downloads only the three pinned CC0 files,
decodes them to stereo 44.1 kHz PCM16 for reliable native seeking, measures real
amplitude/frequency envelopes, and creates a fresh DeviceSQL database plus DAT,
EXT and 2EX analysis. It never reads the personal library or firmware. The Rust
export command writes the FAT32 image. This is an offline fixture builder, not
an alternative emulator host.

Set local `output/desktop-defaults.json` to the generated `USB` directory and
`cc0-demo.img` using absolute `usb_folder`/`usb_image` paths. The desktop and
backend shell honor those defaults; explicit media and a saved shell selection
take precedence. A missing configured image produces an error. Use separate
player states/overlays and the optimized backend for demos.

Detail is 150 samples/second, with blue, RGB and three-band buffers; overviews
are 400/100 blue and 1,200 RGB/three-band columns. The grids use creator-supplied
tempo with phase estimated from low-frequency onsets. These are useful test
fixtures, not rekordbox's proprietary analysis or proof of perfect beat/bar
accuracy. Inspect and adjust the grid before precise mixing qualification.

Browser configuration and WAV type records follow published
[DeviceSQL writer definitions](https://docs.rs/baken-export/latest/src/baken_export/pdb/fixed.rs.html).
Empty tables end at their index page; all database records are generated afresh.
Analysis directories use the player's audio-path hash, verified against stock
v1.45 creating analysis for the CC0 fixture in its private overlay. DAT headers
include the lossless seek table and player-compatible beat/preview constants.

Format references: [database](https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/exports.html)
and [ANLZ](https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html).

Native fixture checks on stock v1.45 (interpreter, audio off) displayed full
precomputed waveforms and beat ticks on the linked pair; the third track also
loaded with a full waveform. Browser titles resolve correctly. The loaded-track
header is currently blank for these WAV fixtures. These checks do not qualify
audio continuity, exact grid alignment or physical hardware. Local evidence is
under `output/cc0-linked-preview-*` and `output/cc0-third-check/`.
