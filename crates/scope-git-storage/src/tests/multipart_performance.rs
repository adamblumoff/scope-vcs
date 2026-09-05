use super::*;

#[tokio::test]
#[ignore = "manual multipart latency experiment"]
async fn measure_multipart_latency() {
    let mut input = vec![0; 64 * 1024 * 1024];
    getrandom::fill(&mut input).unwrap();
    for delay in [0, 100] {
        let fixture = Fixture::new(1024 * 1024, 8 * 1024 * 1024, 2);
        fixture.backend.part_delay_ms.store(delay, Ordering::SeqCst);
        let staged = fixture
            .store
            .ingest(REPOSITORY_ID, input.as_slice(), u64::MAX)
            .await
            .unwrap();
        let (restored, _) = restore_bytes(&fixture.store, &staged.segment)
            .await
            .unwrap();
        assert_eq!(restored, input);
        eprintln!(
            "64MiB incompressible, delay {delay}ms: {:?}",
            staged.timings
        );
    }
}

#[tokio::test]
async fn multipart_overlaps_two_parts_and_preserves_completion_order() {
    let fixture = Fixture::new(32, 128, 1);
    fixture.backend.part_delay_ms.store(10, Ordering::SeqCst);
    fixture.backend.reorder_parts.store(true, Ordering::SeqCst);
    let input = vec![37; 1024];
    let staged = fixture
        .store
        .ingest(REPOSITORY_ID, input.as_slice(), u64::MAX)
        .await
        .unwrap();
    assert_eq!(fixture.backend.peak_parts.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.backend.active_parts.load(Ordering::SeqCst), 0);
    let (restored, _) = restore_bytes(&fixture.store, &staged.segment)
        .await
        .unwrap();
    assert_eq!(restored, input);
}
