use super::*;
use crate::execution::fake::{FakeEcs, TEST_IMAGE};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[tokio::test]
async fn provider_bursts_overlap_within_the_start_bound() {
    let provider = FakeEcs::new().await;
    let mut starts = Provisioning::new(20);
    for attempt in 0..MAX_PROVIDER_STARTS {
        starts.wait_for_slot().await.unwrap();
        let client = provider.client.clone();
        starts.spawn(async move {
            client
                .start(TEST_IMAGE, &format!("attempt_{attempt}"), "token", 86400)
                .await
                .map(|_| ())
                .map_err(|error| anyhow::anyhow!("{error:?}"))
        });
    }
    provider.wait_for("RunTask", MAX_PROVIDER_STARTS).await;
    assert_eq!(provider.peak_starts(), MAX_PROVIDER_STARTS);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), starts.wait_for_slot())
            .await
            .is_err()
    );
    // One completed start opens a slot without waiting for the three slow starts.
    provider.starts.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), starts.wait_for_slot())
        .await
        .unwrap()
        .unwrap();
    let client = provider.client.clone();
    starts.spawn(async move {
        client
            .start(TEST_IMAGE, "attempt_next", "token", 86400)
            .await
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("{error:?}"))
    });
    provider.wait_for("RunTask", MAX_PROVIDER_STARTS + 1).await;
    assert_eq!(provider.peak_starts(), MAX_PROVIDER_STARTS);
    provider.starts.add_permits(MAX_PROVIDER_STARTS);
    starts.finish().await.unwrap();
}

#[tokio::test]
async fn provisioning_respects_a_smaller_configured_capacity() {
    let mut starts = Provisioning::new(1);
    let (release, pending) = tokio::sync::oneshot::channel();
    starts.spawn(async move {
        pending.await.unwrap();
        Ok(())
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(30), starts.wait_for_slot())
            .await
            .is_err()
    );
    release.send(()).unwrap();
    starts.wait_for_slot().await.unwrap();
    starts.finish().await.unwrap();
}

#[tokio::test]
async fn one_failed_start_does_not_drop_other_reserved_starts() {
    let mut starts = Provisioning::new(4);
    let completed = Arc::new(AtomicBool::new(false));
    starts.spawn(async { anyhow::bail!("provider failure") });
    let succeeded = completed.clone();
    starts.spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        succeeded.store(true, Ordering::SeqCst);
        Ok(())
    });
    assert!(starts.finish().await.is_err());
    assert!(completed.load(Ordering::SeqCst));
}
