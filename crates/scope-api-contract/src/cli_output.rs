use serde::{Deserialize, Serialize};

pub const CLI_OUTPUT_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CliSuccessEnvelope<T> {
    pub version: u32,
    pub command: String,
    pub result: T,
}

impl<T> CliSuccessEnvelope<T> {
    pub fn new(command: impl Into<String>, result: T) -> Self {
        Self {
            version: CLI_OUTPUT_VERSION,
            command: command.into(),
            result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_envelope_has_one_stable_version_and_command() {
        let envelope = CliSuccessEnvelope::new(
            "request.submit",
            serde_json::json!({"request": {"id": "req_one"}}),
        );
        let value = serde_json::to_value(envelope).unwrap();

        assert_eq!(value["version"], 1);
        assert_eq!(value["command"], "request.submit");
        assert_eq!(value["result"]["request"]["id"], "req_one");
    }
}
