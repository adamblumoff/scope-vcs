use anyhow::Context as _;
use std::{
    process, thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(crate) fn arm(deadline_unix: u64) -> anyhow::Result<()> {
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs();
    let remaining = remaining_seconds(deadline_unix, now_unix);
    thread::Builder::new()
        .name("attempt-deadline".to_string())
        .spawn(move || {
            if remaining > 0 {
                thread::sleep(Duration::from_secs(remaining));
            }
            eprintln!("run attempt reached its absolute deadline");
            process::exit(124);
        })
        .context("start attempt deadline watchdog")?;
    Ok(())
}

fn remaining_seconds(deadline_unix: u64, now_unix: u64) -> u64 {
    deadline_unix.saturating_sub(now_unix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_deadlines_fire_immediately() {
        assert_eq!(remaining_seconds(99, 100), 0);
    }

    #[test]
    fn future_deadlines_preserve_the_absolute_remaining_time() {
        assert_eq!(remaining_seconds(200, 125), 75);
    }
}
