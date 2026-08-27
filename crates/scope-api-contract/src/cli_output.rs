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
