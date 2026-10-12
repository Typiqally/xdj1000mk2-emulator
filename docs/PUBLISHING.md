# Publishing the emulator

The emulator owns the Rust backend/UI and retained QEMU/C DSP source. Firmware
modification tools belong to the separate firmware repository. The publication
includes current source changes; it does not distribute vendor updates, music,
USB exports, user settings, diagnostic logs, generated DSP modules or build caches.
The two reviewed README screenshots are included as documentation assets.

## Updating the existing repository

The primary checkout is the active repository on `main`, tracking
`origin/main` at [Typiqally/xdj1000mk2-emulator](https://github.com/Typiqally/xdj1000mk2-emulator).
Make ordinary source/documentation changes here, run the appropriate checks in
`AGENTS.md` and `python3 scripts/check_publication.py check`, then commit and push
through that configured remote. A separate publishing checkout is unnecessary.

The 2026-10-12 cleanup removed redundant publishing checkouts and disposable
build caches. Historical source, failure evidence and identities are retained
locally as described in [engineering findings](engineering-findings.md).
Do not upload `output/`, local recovery bundles, firmware or player state.

## Exporting a new source snapshot

Use the following only when a separate source export is explicitly needed.
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

For a separately requested new repository, configure its intended remote on
that snapshot. Keep the research workspace's state, build output and uncommitted
diagnostics local. The script neither uploads nor pushes. The existing emulator
repository does not need to be recreated.

Pattern checks detect recognizable credentials and reject unexpected binary or
local-state paths; review the selected source before publication. They cannot
prove that every possible secret is absent. The README images are real Rust emulator captures with a synthetic test file;
their provenance is recorded in `docs/assets/README.md`. Screenshots do not
qualify audio or physical hardware.

Imported native source and patch context preserve their original whitespace.
Do not normalize those files merely to silence initial-import diff warnings.
