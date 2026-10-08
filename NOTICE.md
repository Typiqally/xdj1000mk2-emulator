# Source and license notices

This repository separates the native emulator from the XDJ firmware research
workspace and replaces its Python host backend with Rust. The native QEMU board
and DSP sources and patches are preserved snapshots. Their original
per-file copyright, SPDX and third-party notices are retained. The Rust update
container parser derives from that workspace's bounded Rust SH4 harness;
backend behavior follows the corresponding host modules and hardware inventory.

The originating workspace baseline revision was
`34e6149fca35d95096103e7a9a027e4e69447884`; the native snapshot includes working
source changes present at extraction. The manifest in
`docs/native-source-manifest.json` records file hashes of the included source.
The baseline revision alone does not identify those working changes.

Original Rust backend, desktop UI and catalog code under `src/` and `crates/`
are distributed under MIT; see LICENSE-MIT. README visuals are actual emulator
UI captures; their provenance is in `docs/assets/README.md`. Original host UI
material is MIT. Vendor firmware-rendered graphics, trademarks and other
third-party content visible in the captures retain their respective ownership
and are not relicensed by LICENSE-MIT.
Native components retain their existing per-file licenses, including GPL;
the GPL-2.0 text in LICENSE is preserved. This does not relicense QEMU, DSP,
binutils-derived tables or other third-party source. The QEMU archive downloaded
by the builder contains its own COPYING files and third-party component notices.
Preserve them in distributions. Cargo dependencies retain their own licenses.

No Pioneer/AlphaTheta firmware updates, extracted firmware, music, USB exports,
service manuals, or user recordings are included. Users supply their own
firmware and media. No affiliation or endorsement is implied.
