use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
#[cfg_attr(any(test, feature = "ts"), ts(rename_all = "snake_case"))]
pub enum RequestQueueSection {
    YourWork,
    Ready,
    Completed,
}
