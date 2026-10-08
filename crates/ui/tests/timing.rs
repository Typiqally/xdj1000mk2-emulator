use serde_json::json;
#[test]
fn diagnostic_input_timestamps_are_opt_in_and_roundtrip_is_measurable() {
    let command = json!({"action":"button","name":"cue","down":true});
    assert_eq!(
        xdj_ui::timing::stamp(command.clone(), false, "host_sent_unix_us"),
        command
    );
    let stamped = xdj_ui::timing::stamp(command, true, "host_sent_unix_us");
    assert!(stamped["host_sent_unix_us"].as_u64().unwrap() > 0);
    let reply = xdj_ui::timing::stamp(
        json!({"event":"input-ack","command":stamped}),
        true,
        "host_received_unix_us",
    );
    assert!(
        reply["host_received_unix_us"].as_u64().unwrap()
            >= reply["command"]["host_sent_unix_us"].as_u64().unwrap()
    );
}
