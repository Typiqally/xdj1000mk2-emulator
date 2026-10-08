use serde_json::json;
use xdj_backend::validation::{assess, check_pcm, host_bins, screen_state};
#[test]
fn screenshots_require_native_loaded_track_and_time() {
    let state = screen_state(
        &json!([{"text":"12","x":0.17,"y":0.77},{"text":"001:23.456","x":0.4,"y":0.8},{"text":"Test Tone","x":0.4,"y":0.18}]),
    );
    assert_eq!(state["loaded"], true);
    assert_eq!(state["remaining_seconds"], 83.456);
    assert_eq!(screen_state(&json!([]))["loaded"], false);
}
#[test]
fn host_statistics_reject_missing_and_lost_callback_intervals() {
    assert!(host_bins("", 0, 1).is_err());
    let report=host_bins("xdj-output origin-us 1000000\nxdj-output second 1 callbacks 2 frames 44100 underruns 0 missed 0\nxdj-output second 2 callbacks 2 frames 44099 underruns 1 missed 1",2000000,4000000).unwrap();
    assert_eq!(report["covered"], true);
    assert_eq!(report["underruns"], 1);
    assert_eq!(report["missed_frames"], 1);
}
#[test]
fn tone_samples_detect_discontinuities_and_incomplete_capture() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pcm");
    let mut bytes = Vec::new();
    for n in 0..44100 {
        for hz in [440., 880.] {
            let value = (6000. * (2. * std::f64::consts::PI * hz * f64::from(n) / 44100.).sin())
                .round_ties_even() as i32;
            bytes.extend((value << 16).to_le_bytes());
        }
    }
    std::fs::write(&file, &bytes).unwrap();
    assert_eq!(
        check_pcm(&file, 0, 44100, true).unwrap()["continuous"],
        true
    );
    bytes[8000..8016].fill(0);
    std::fs::write(&file, &bytes).unwrap();
    assert_eq!(
        check_pcm(&file, 0, 44100, true).unwrap()["continuous"],
        false
    );
    assert!(check_pcm(&file, 0, 44101, true).is_err());
}
#[test]
fn readiness_never_forgives_boot_losses_or_missing_evidence() {
    let first = json!({"running":true,"stop":false,"dropped_frames":0,"queue_recoveries":0,"serializer_underruns":0});
    let mut report = json!({"first":first,"last":first,"audio":{"continuous":true,"seconds":65.},"host":{"covered":true,"underruns":0,"missed_frames":0,"seconds":vec![0;63]},"native_time_advance_seconds":64.,"controls":{"queue_continuity":"pass","first":first}});
    assert!(assess(&report, 60.));
    report["first"]["dropped_frames"] = json!(1);
    assert!(!assess(&report, 60.));
    assert!(!assess(&json!({}), 60.));
}
