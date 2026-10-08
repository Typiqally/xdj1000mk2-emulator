use eframe::egui;
use serde_json::json;
use std::fs;
use xdj_ui::app::{DeckApp, Options};

#[test]
fn archived_firmware_is_available_before_starting_a_backend() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("firmware");
    fs::create_dir_all(archive.join("v145")).unwrap();
    fs::write(archive.join("v145/XDJ1KMK2.UPD"), b"fixture").unwrap();
    fs::write(
        archive.join("manifest.json"),
        json!({"versions":[{"version":"1.45","upd_sha256":"test"}]}).to_string(),
    )
    .unwrap();
    let app = DeckApp::offline(Options {
        root: dir.path().into(),
        ..Options::default()
    });
    assert_eq!(app.player.catalog.len(), 1);
    assert_eq!(app.player.catalog[0]["version"], "1.45");
    assert_eq!(app.report()["backend_pid"], serde_json::Value::Null);
}

#[test]
fn desktop_menus_have_visible_empty_states_and_dispatch_existing_commands() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = DeckApp::offline(Options {
        root: dir.path().into(),
        ..Options::default()
    });
    let menus = app.menu_entries();
    assert_eq!(
        menus.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(),
        ["File", "Player", "View"]
    );
    assert_eq!(menus[0].children[1].label, "Recent Firmware");
    assert_eq!(menus[0].children[1].children[0].label, "No recent firmware");
    assert_eq!(
        menus[0].children[2].children[0].label,
        "No archived firmware available"
    );
    assert!(!menus[0].children[2].children[0].enabled);
    let ctx = egui::Context::default();
    app.activate_menu(json!({"action":"view","view":"player"}), &ctx);
    assert_eq!(app.options.view, "player");
    app.activate_menu(json!({"action":"mute","muted":false}), &ctx);
    assert_eq!(app.report()["muted"], false);
    assert!(
        app.take_commands()
            .contains(&json!({"action":"mute","muted":false}))
    );
    app.player.ready = true;
    app.activate_menu(json!({"action":"stop"}), &ctx);
    assert!(!app.player.ready);
    assert!(app.take_commands().contains(&json!({"action":"stop"})));
}

#[test]
fn installed_app_resolves_its_repository_and_bundled_backend_from_resources() {
    let dir = tempfile::tempdir().unwrap();
    let resources = dir.path().join("App.app/Contents/Resources");
    fs::create_dir_all(&resources).unwrap();
    let root = dir.path().join("repository");
    fs::create_dir(&root).unwrap();
    let executable = dir.path().join("App.app/Contents/MacOS/XDJEmulator");
    let backend = resources.join("xdj-backend");
    fs::write(
        resources.join("config.json"),
        json!({"root":root,"backend":backend}).to_string(),
    )
    .unwrap();
    let (actual_root, actual_backend) =
        Options::paths_for_executable(&executable, dir.path()).unwrap();
    assert_eq!(actual_root, root);
    assert_eq!(actual_backend, backend);
    fs::write(resources.join("config.json"), b"invalid").unwrap();
    assert!(Options::paths_for_executable(&executable, dir.path()).is_err());
}

#[test]
fn ignored_local_usb_defaults_apply_without_overriding_explicit_media() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir(root.join("output")).unwrap();
    let image = root.join("local-usb.img");
    fs::write(&image, b"local image").unwrap();
    fs::write(
        root.join("output/desktop-defaults.json"),
        json!({"usb_folder":root,"usb_image":image}).to_string(),
    )
    .unwrap();
    let options = Options::parse(["--root".into(), root.display().to_string()]).unwrap();
    assert!(
        options
            .backend_args
            .windows(2)
            .any(|pair| pair[0] == "--usb-image" && pair[1] == image.display().to_string())
    );
    let explicit = Options::parse([
        "--root".into(),
        root.display().to_string(),
        "--".into(),
        "--usb-image".into(),
        "explicit.img".into(),
    ])
    .unwrap();
    assert_eq!(explicit.backend_args, ["--usb-image", "explicit.img"]);
}

#[test]
fn explicit_backend_survives_a_later_root_argument() {
    let dir = tempfile::tempdir().unwrap();
    let options = Options::parse([
        "--backend".into(),
        "selected-backend".into(),
        "--root".into(),
        dir.path().display().to_string(),
    ])
    .unwrap();
    assert_eq!(
        options.backend,
        std::path::PathBuf::from("selected-backend")
    );
}
