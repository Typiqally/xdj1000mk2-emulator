use serde_json::{Value, json};
pub fn stamp(mut value: Value, enabled: bool, field: &str) -> Value {
    if enabled && let Some(object) = value.as_object_mut() {
        object.insert(
            field.into(),
            json!(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros() as u64
            ),
        );
    }
    value
}
