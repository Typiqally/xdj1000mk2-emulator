use serde_json::json;
use xdj_ui::{
    input::{Input, lcd_point},
    model::Player,
    rfb::Framebuffer,
};

#[test]
fn release_on_focus_loss_covers_button_touch_and_jog() {
    let mut input = Input::default();
    assert_eq!(
        input.button("cue", true),
        Some(json!({"action":"button","name":"cue","down":true}))
    );
    assert_eq!(input.button("cue", true), None);
    input.touch(799, 479, true);
    input.button("jog-touch", true);
    assert_eq!(input.release(), Some(json!({"action":"release"})));
    assert_eq!(input.release(), None);
    assert!(input.button("cue", true).is_some());
}

#[test]
fn touchscreen_letterbox_edges_map_to_native_pixels() {
    assert_eq!(
        lcd_point([24.0, 27.0, 800.0, 480.0], [24.0, 27.0]),
        Some((0, 0))
    );
    assert_eq!(
        lcd_point([24.0, 27.0, 800.0, 480.0], [824.0, 507.0]),
        Some((799, 479))
    );
    assert_eq!(lcd_point([24.0, 27.0, 800.0, 480.0], [23.9, 200.0]), None);
    assert_eq!(lcd_point([0.0, 0.0, 0.0, 480.0], [0.0, 0.0]), None);
}

#[test]
fn invalid_rectangles_never_publish_partial_display() {
    let mut fb = Framebuffer::new(3, 2).unwrap();
    fb.raw(1, 0, 2, 1, &[0, 0, 255, 0, 0, 255, 0, 0]).unwrap();
    assert_eq!(&fb.rgba()[4..12], &[255, 0, 0, 255, 0, 255, 0, 255]);
    let before = fb.rgba().to_vec();
    assert!(fb.raw(2, 1, 2, 1, &[0; 8]).is_err());
    assert!(fb.raw(0, 0, 1, 1, &[0; 3]).is_err());
    assert_eq!(fb.rgba(), before);
    assert!(Framebuffer::new(0, 2).is_err());
    assert!(Framebuffer::new(4097, 2).is_err());
}

#[test]
fn firmware_transition_disables_controls_and_recoverable_error_restores_previous() {
    let mut player = Player::default();
    player.event(&json!({"event":"running","vnc":"/tmp/display.sock","firmware":{"version":"1.45"},"source":"/firmware.upd"}));
    assert!(player.ready);
    player.event(&json!({"event":"loading","message":"Copying virtual USB…"}));
    assert!(!player.ready);
    player.event(&json!({"event":"error","message":"Invalid image","recoverable":true}));
    assert!(player.ready);
    player.event(&json!({"event":"stopped"}));
    assert!(!player.ready);
    player.event(&json!({"event":"error","message":"No engine","recoverable":true}));
    assert!(!player.ready);
}
