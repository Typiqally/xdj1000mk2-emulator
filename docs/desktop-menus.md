# Desktop menu and launcher migration — 2026-10-08

The installed macOS app is `~/Applications/XDJ-1000MK2 Emulator.app`, built from
`xdj-ui` and a bundled Rust host backend. Its title has no implementation suffix.
`build-app` builds that UI with no Swift comparison code.
The app reads its repository/backend paths from bundle resources.

File, Player and View menus share commands with the deck. File includes Recent
Firmware (ten original source paths, missing files disabled, atomic history,
clear action) and Archived Firmware (local images available before backend
startup). Media and runtime actions release held controls. Windows/Linux use
the same tree in the application window. Menu callbacks wake the render thread.

Firmware/media stay local and ignored. Desktop USB defaults are read from
`output/desktop-defaults.json`; `usb_folder` selects the dialog's directory and
`usb_image` attaches its existing image on firmware startup. Explicit media
arguments override the default. USB writes still use per-collection overlays.
No firmware images, music, USB content or user-specific default paths are committed.

## Verification

- 80 workspace Rust tests passed; formatting and workspace clippy passed with
  warnings denied. Windows/Linux UI all-target compile checks passed.
- The opt-in macOS desktop integration test passed with a private synthetic
  JSON/RFB peer, verifying installed AppKit menus, archive selection, view/mute
  callbacks, and clearing private recent history. Run with
  `cargo test -p xdj-ui --test native_menus -- --ignored` on a desktop.
- Instrumented UI/catalog tests and that desktop run reached 85.90% line coverage.
- The installed release app exposed File/Player/View menus and ten verified
  archived images before starting any backend. Native view, mute, logs and keys
  actions passed; screenshots preserve the deck layout.
- Native archive selection booted original 1.45 with audio off, interpreter mode
  and disposable state; native Stop ended that owned player. Input SHA-256:
  `6fd2a5002b297b86fb8a5931d3debf1b607ff6abaf9d49cd737d3a530002dbcd`.
  Engine SHA-256:
  `f9801338fb81749c3c295a84df603e05a1ca3d4f2ec5e6aaef89b3604461b617`.
  This checks menus, input and boot, not audio/playback/hardware qualification.
- Three stale Swift app bundles and five legacy command launchers were archived,
  hash-verified, removed and unregistered individually. Spotlight finds the one
  installed Rust app. Firmware, media, state and diagnostic runs remain intact.

Local evidence: `output/menus-*-qa`, `output/menus-*.log`,
`output/menus-ui-coverage-summary.txt`, `output/archive-migration-verification.json`
and `output/legacy-recovery/stale-launchers-removal.json`. The obsolete Swift launcher recovery archive was subsequently deleted; its original
hash remains in `output/legacy-recovery/swift-removal.json`. No browser session was opened.
