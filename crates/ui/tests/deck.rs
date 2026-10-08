use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, Rect, Vec2};
use serde_json::json;
use xdj_ui::app::{DeckApp, Options};

#[test]
fn native_pointer_hold_release_and_focus_loss_reach_the_backend_contract() {
    let ctx = egui::Context::default();
    let mut app = DeckApp::offline(Options::default());
    app.player
        .event(&json!({"event":"running","vnc":"fixture"}));
    let mut time = 0.0;
    let mut draw = |app: &mut DeckApp, events: Vec<Event>, focused: bool| {
        time += 0.02;
        let mut raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 798.0))),
            time: Some(time),
            events,
            focused,
            ..Default::default()
        };
        raw.viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .focused = Some(focused);
        let mut output = ctx.run_ui(raw, |ui| app.show(ui));
        output.textures_delta.clear();
    };
    draw(&mut app, vec![], true);
    let cue = app.control_rect("cue").unwrap().center();
    draw(
        &mut app,
        vec![
            Event::PointerMoved(cue),
            Event::PointerButton {
                pos: cue,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        true,
    );
    assert!(
        app.take_commands()
            .contains(&json!({"action":"button","name":"cue","down":true}))
    );
    draw(&mut app, vec![], false);
    assert!(app.take_commands().contains(&json!({"action":"release"})));
    draw(
        &mut app,
        vec![Event::PointerButton {
            pos: cue,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        true,
    );
}

#[test]
fn empty_player_draws_and_disables_native_controls_in_both_layouts() {
    for view in ["firmware", "player"] {
        let ctx = egui::Context::default();
        let mut app = DeckApp::offline(Options {
            view: view.into(),
            ..Default::default()
        });
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
        assert!(!app.player.ready);
        assert!(app.take_commands().is_empty());
        assert!(app.control_rect("play").is_some());
        assert!(app.control_rect("load").is_some());
    }
}

#[test]
fn tempo_top_is_minimum_backend_value_and_double_click_centres() {
    let ctx = egui::Context::default();
    let mut app = DeckApp::offline(Options::default());
    app.player.event(&json!({"event":"running"}));
    let draw = |app: &mut DeckApp, events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 798.))),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        output.textures_delta.clear();
    };
    draw(&mut app, vec![]);
    let fader = app
        .control_rect("fader")
        .expect("The whole fader has an accessible hit region");
    let point = Pos2::new(fader.center().x, fader.top() + 8.);
    draw(
        &mut app,
        vec![
            Event::PointerMoved(point),
            Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    let commands = app.take_commands();
    assert!(
        commands
            .iter()
            .any(|v| v["action"] == "pitch" && v["value"].as_i64().unwrap() < 1000),
        "Top must reduce pitch, commands={commands:?}"
    );
}

#[test]
fn mouse_button_stays_held_across_redraws_without_release_or_repeat() {
    let ctx = egui::Context::default();
    let mut app = DeckApp::offline(Options::default());
    app.player.event(&json!({"event":"running"}));
    let mut time = 0.;
    let mut draw = |app: &mut DeckApp, events| {
        time += 0.02;
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 798.))),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        output.textures_delta.clear();
    };
    draw(&mut app, vec![]);
    let cue = app.control_rect("cue").unwrap().center();
    draw(
        &mut app,
        vec![
            Event::PointerMoved(cue),
            Event::PointerButton {
                pos: cue,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    app.take_commands();
    for _ in 0..5 {
        draw(&mut app, vec![]);
        assert!(
            app.take_commands().is_empty(),
            "Holding a button must survive repaint"
        );
    }
    draw(
        &mut app,
        vec![Event::PointerButton {
            pos: cue,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(
        app.take_commands(),
        vec![json!({"action":"button","name":"cue","down":false})]
    );
}

#[test]
fn native_keyboard_shortcuts_still_work_after_clicking_a_deck_button() {
    let ctx = egui::Context::default();
    let mut app = DeckApp::offline(Options::default());
    app.player.event(&json!({"event":"running"}));
    let mut time = 0.;
    let mut draw = |app: &mut DeckApp, events| {
        time += 0.1;
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 798.))),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        output.textures_delta.clear();
    };
    draw(&mut app, vec![]);
    let cue = app.control_rect("cue").unwrap().center();
    for pressed in [true, false] {
        draw(
            &mut app,
            vec![
                Event::PointerMoved(cue),
                Event::PointerButton {
                    pos: cue,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    app.take_commands();
    draw(
        &mut app,
        vec![Event::Key {
            key: egui::Key::Space,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(
        app.take_commands(),
        vec![json!({"action":"button","name":"play","down":true})]
    );
    draw(
        &mut app,
        vec![Event::Key {
            key: egui::Key::Space,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(
        app.take_commands(),
        vec![json!({"action":"button","name":"play","down":false})]
    );
}

#[test]
fn reference_deck_retains_cap_geometry_and_external_legends() {
    let ctx = egui::Context::default();
    let mut app = DeckApp::offline(Options::default());
    let mut output = ctx.run_ui(
        egui::RawInput {
            // Leave room for Linux/Windows' in-window menu as well as the
            // header and footer, so these assertions exercise 1x geometry.
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 900.))),
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    let labels: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect();
    output.textures_delta.clear();
    for (name, size) in [
        ("time", [28., 28.]),
        ("load", [104., 104.]),
        ("reloop", [40., 40.]),
        ("master-tempo", [30., 30.]),
    ] {
        assert_eq!(
            app.control_rect(name).unwrap().size(),
            Vec2::from(size),
            "Reference cap size: {name}"
        );
    }
    for text in [
        "Track filter",
        "Touch / Release",
        "Panel lamps",
        "Firmware",
        "Open firmware to start · ⌘F",
    ] {
        assert!(
            labels.iter().any(|s| s == text),
            "Missing reference legend {text}"
        );
    }
    assert!(!labels.iter().any(|s| s == "JOG" || s == "PLAY\nPAUSE"));
}
