# Contributing

Read `AGENTS.md` and the emulator skill before changing emulator behavior.
Edit maintained source under `src/`, `crates/` and `native/`; generated engine
source, firmware, media and player state stay local.

Run the workspace Rust formatting, Clippy and tests documented in `README.md`.
For native changes, also run the Python native component suite. Those tests use
synthetic inputs and private builds; they do not require a vendor update.
Boot, screenshots and synthetic tests do not qualify continuous audio or
physical hardware behavior.

Preserve per-file native copyright and license notices. Original Rust code uses
MIT; third-party/native source keeps its existing terms. Never attach firmware,
USB exports, music, personal settings, credentials or private captures to issues
or pull requests. Use synthetic reproductions where possible.

For a behavior-changing pull request, provide a short manual test plan and leave
the personal test checkbox unchecked until the author has run it. Agent tests
and green CI do not replace that confirmation before merging.

See `docs/PUBLISHING.md` for source-only publication checks.
