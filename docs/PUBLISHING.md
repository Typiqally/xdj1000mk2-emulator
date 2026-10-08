# Publishing the emulator

The emulator owns the Rust backend/UI and retained QEMU/C DSP source. Firmware
modification tools belong to the separate firmware repository. The publication
includes current source changes; it does not distribute vendor updates, music,
USB exports, user settings, diagnostic logs, generated DSP modules or build caches.
The two reviewed README screenshots are included as documentation assets.

Prepare a new source snapshot from the repository root:

```sh
python3 scripts/check_publication.py export --out output/github-source/repository
```

The destination must not exist. Export copies allowlisted current source,
documentation, license notices, CI and the two hash-pinned README screenshots. It does not copy
Git history or change the research index. Historical source manifests in `docs/`
remain evidence of their original snapshots, not qualification of this snapshot.

From the exported repository:

```sh
git init -b main
git add .
python3 scripts/check_publication.py check
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
python3 -m unittest discover -s tests/native -v
git diff --cached --check -- . ':!native'
git commit -m "Initial emulator source publication"
```

Create a private GitHub repository named `xdj1000mk2-emulator`, add its remote
to this snapshot and push `main`. Keep the research workspace's state, build
output and uncommitted diagnostics local. The script neither uploads nor pushes.

Pattern checks detect recognizable credentials and reject unexpected binary or
local-state paths; review the selected source before publication. They cannot
prove that every possible secret is absent. The README images are real Rust emulator captures with a synthetic test file;
their provenance is recorded in `docs/assets/README.md`. Screenshots do not
qualify audio or physical hardware.

Imported native source and patch context preserve their original whitespace.
Do not normalize those files merely to silence initial-import diff warnings.
