# Source publication checks — 2026-10-08

This records the clean emulator source snapshot prepared for private GitHub
publication. It includes the current pending Rust/native source changes, with
Rust formatting applied only in the publishing snapshot. It is a development
source publication, not a binary release or playback/hardware qualification.

| Check | Result |
| --- | --- |
| Publication allowlist, credential patterns, artwork hash and README links | Pass |
| `cargo fmt --all --check` | Pass |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Pass |
| `cargo test --locked --workspace --no-fail-fast` | 85 passed, 0 failed on the complete rerun |
| `python3 -m unittest discover -s tests/native -v` | 66 tests run; 13 failures and 2 errors |

The first Rust attempt failed in macOS Vision OCR with
`TextRecognition.CRImageReaderError error 1`; that test passed on the full rerun.
The result does not establish that the platform OCR dependency is consistently
available.

Multiple native fixtures provide module ABI 12 profiles while the current core
header and generator require ABI 14. The suite also reports HPI arbitration,
generated-prelude/rounding compilation, and adaptive JIT assertion failures.
These are retained as failing evidence; the publication work does not change
runtime behavior or weaken assertions to make the suite green. Native CI may
therefore fail until those source/fixture changes are completed.

Full local logs are in ignored `output/github-source/rust-workspace-check.log`
and `output/github-source/native-component-check.log`. Firmware, media, player
state, compiled modules and those local logs are excluded from publication.
