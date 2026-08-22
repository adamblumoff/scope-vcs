use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static REPLICA_ID: OnceLock<String> = OnceLock::new();

pub(crate) fn replica_id() -> &'static str {
    REPLICA_ID
        .get_or_init(|| {
            std::env::var("RAILWAY_REPLICA_ID")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "local".to_string())
        })
        .as_str()
}

pub(crate) fn request_trace_id() -> String {
    let mut random = [0_u8; 8];
    if getrandom::fill(&mut random).is_ok() {
        return hex::encode(random);
    }
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{}-{sequence}", std::process::id())
}
