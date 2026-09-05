use crate::RouterConfig;
use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{net::lookup_host, sync::Notify, task::AbortHandle};

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

type ResolveFuture =
    Pin<Box<dyn Future<Output = anyhow::Result<Vec<SocketAddr>>> + Send + 'static>>;
type Resolver = dyn Fn(Arc<str>) -> ResolveFuture + Send + Sync;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Backend {
    pub(crate) identity: String,
    pub(crate) address: SocketAddr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiscoveryFreshness {
    Fresh,
    Stale,
}

#[derive(Clone, Debug)]
pub(crate) struct DiscoveredBackends {
    pub(crate) backends: Vec<Backend>,
    pub(crate) freshness: DiscoveryFreshness,
    pub(crate) age: Duration,
}

#[derive(Clone)]
pub struct BackendDiscovery {
    authority: Arc<str>,
    refresh_after: Duration,
    max_stale_age: Duration,
    resolver: Arc<Resolver>,
    state: Arc<DiscoveryState>,
}

#[derive(Default)]
struct DiscoveryState {
    cached: Mutex<CachedDiscovery>,
    refreshed: Notify,
}

#[derive(Default)]
struct CachedDiscovery {
    snapshot: Option<Snapshot>,
    last_attempt_at: Option<Instant>,
    last_error: Option<Arc<str>>,
    refresh: Option<AbortHandle>,
}

impl Drop for DiscoveryState {
    fn drop(&mut self) {
        if let Some(task) = self
            .cached
            .get_mut()
            .expect("discovery lock")
            .refresh
            .take()
        {
            task.abort();
        }
    }
}

struct Snapshot {
    resolved_at: Instant,
    backends: Vec<Backend>,
}

impl BackendDiscovery {
    pub fn new(config: &RouterConfig) -> Self {
        Self::with_resolver(config, Arc::new(system_resolver))
    }

    fn with_resolver(config: &RouterConfig, resolver: Arc<Resolver>) -> Self {
        Self {
            authority: Arc::from(config.backend_authority.as_str()),
            refresh_after: config.dns_refresh,
            max_stale_age: config.dns_max_stale,
            resolver,
            state: Arc::new(DiscoveryState::default()),
        }
    }

    pub(crate) async fn backends(&self) -> anyhow::Result<DiscoveredBackends> {
        loop {
            // Register before inspecting the state so discovery cannot miss refresh completion.
            let refreshed = self.state.refreshed.notified();
            tokio::pin!(refreshed);
            refreshed.as_mut().enable();
            {
                let mut cached = self.state.cached.lock().expect("discovery lock");
                let now = Instant::now();
                let due = cached.last_attempt_at.is_none_or(|attempt| {
                    now.saturating_duration_since(attempt) >= self.refresh_after
                });
                if due && cached.refresh.is_none() {
                    cached.last_attempt_at = Some(now);
                    cached.refresh = Some(self.spawn_refresh());
                }
                if let Some(snapshot) = &cached.snapshot {
                    let age = now.saturating_duration_since(snapshot.resolved_at);
                    if age <= self.max_stale_age {
                        return Ok(DiscoveredBackends {
                            backends: snapshot.backends.clone(),
                            freshness: if age < self.refresh_after {
                                DiscoveryFreshness::Fresh
                            } else {
                                DiscoveryFreshness::Stale
                            },
                            age,
                        });
                    }
                    if cached.refresh.is_none() {
                        let error = cached
                            .last_error
                            .as_deref()
                            .unwrap_or("discovery unavailable");
                        return Err(expired_error(&self.authority, age, error));
                    }
                }
                if cached.refresh.is_none() {
                    return Err(anyhow::anyhow!(
                        "{}",
                        cached
                            .last_error
                            .as_deref()
                            .unwrap_or("discovery unavailable")
                    ));
                }
            }
            refreshed.await;
        }
    }

    fn spawn_refresh(&self) -> AbortHandle {
        let state = Arc::downgrade(&self.state);
        let resolver = Arc::clone(&self.resolver);
        let authority = Arc::clone(&self.authority);
        tokio::spawn(async move {
            let result = match tokio::time::timeout(RESOLVE_TIMEOUT, resolver(Arc::clone(&authority))).await {
                Ok(result) => result.and_then(normalize_resolved),
                Err(_) => Err(anyhow::anyhow!("DNS resolution timed out")),
            };
            // No strong state reference survives the DNS await. Dropping discovery aborts this task.
            let Some(state) = state.upgrade() else { return; };
            {
                let mut cached = state.cached.lock().expect("discovery lock");
                cached.refresh = None;
                cached.last_attempt_at = Some(Instant::now());
                match result {
                    Ok(backends) => {
                        let topology_changed = cached.snapshot.as_ref().is_none_or(|snapshot| snapshot.backends != backends);
                        tracing::info!(%authority, backend_count = backends.len(), topology_changed, discovery_state = "fresh", "refreshed API replica discovery");
                        cached.snapshot = Some(Snapshot { resolved_at: Instant::now(), backends });
                        cached.last_error = None;
                    }
                    Err(error) => {
                        tracing::warn!(%authority, %error, has_snapshot = cached.snapshot.is_some(), "API replica discovery refresh failed");
                        cached.last_error = Some(Arc::from(error.to_string()));
                    }
                }
            }
            state.refreshed.notify_waiters();
        }).abort_handle()
    }

    #[cfg(test)]
    pub(crate) fn fixed(backends: Vec<SocketAddr>) -> Self {
        Self::fixed_with_age(
            backends,
            Duration::from_secs(3600),
            Duration::from_secs(7200),
            Duration::ZERO,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn fixed_with_age(
        backends: Vec<SocketAddr>,
        refresh_after: Duration,
        max_stale_age: Duration,
        age: Duration,
        last_error: Option<&str>,
    ) -> Self {
        let now = Instant::now();
        Self {
            authority: Arc::from("fixed.invalid:8080"),
            refresh_after,
            max_stale_age,
            resolver: Arc::new(system_resolver),
            state: Arc::new(DiscoveryState {
                cached: Mutex::new(CachedDiscovery {
                    snapshot: Some(Snapshot {
                        resolved_at: now - age,
                        backends: normalize(backends),
                    }),
                    last_attempt_at: Some(now),
                    last_error: last_error.map(Arc::from),
                    refresh: None,
                }),
                refreshed: Notify::new(),
            }),
        }
    }
}

fn expired_error(authority: &str, age: Duration, error: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{authority} API replica discovery snapshot expired after {} ms: {error}",
        age.as_millis()
    )
}

fn system_resolver(authority: Arc<str>) -> ResolveFuture {
    Box::pin(async move { Ok(lookup_host(authority.as_ref()).await?.collect::<Vec<_>>()) })
}

fn normalize_resolved(addresses: Vec<SocketAddr>) -> anyhow::Result<Vec<Backend>> {
    let backends = normalize(addresses);
    if backends.is_empty() {
        anyhow::bail!("DNS did not resolve to an API replica");
    }
    Ok(backends)
}

fn normalize(mut addresses: Vec<SocketAddr>) -> Vec<Backend> {
    if addresses.iter().any(SocketAddr::is_ipv6) {
        addresses.retain(SocketAddr::is_ipv6);
    }
    addresses.sort_unstable();
    addresses.dedup();
    addresses
        .into_iter()
        .map(|address| Backend {
            identity: address.to_string(),
            address,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex};

    fn config() -> RouterConfig {
        RouterConfig {
            backend_authority: "scripted.invalid:8080".into(),
            dns_refresh: Duration::from_secs(10),
            dns_max_stale: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(1),
            read_timeout: Duration::from_secs(1),
            read_replicas: 1,
            upload_pack_replay_max_bytes: 1024,
        }
    }

    fn scripted(outcomes: Vec<Result<Vec<SocketAddr>, &str>>) -> BackendDiscovery {
        let outcomes = Arc::new(Mutex::new(
            outcomes
                .into_iter()
                .map(|outcome| outcome.map_err(str::to_string))
                .collect::<VecDeque<_>>(),
        ));
        let resolver = Arc::new(move |_authority: Arc<str>| {
            let outcome = outcomes
                .lock()
                .expect("scripted resolver lock")
                .pop_front()
                .expect("scripted resolver outcome");
            Box::pin(async move { outcome.map_err(anyhow::Error::msg) }) as ResolveFuture
        });
        BackendDiscovery::with_resolver(&config(), resolver)
    }

    async fn age_snapshot(discovery: &BackendDiscovery, age: Duration) {
        let mut cached = discovery.state.cached.lock().unwrap();
        let current = cached.snapshot.as_mut().expect("discovery snapshot");
        current.resolved_at = Instant::now() - age;
        cached.last_attempt_at = Some(current.resolved_at);
    }

    async fn wait_for_refresh(discovery: &BackendDiscovery) {
        loop {
            let done = discovery.state.refreshed.notified();
            tokio::pin!(done);
            done.as_mut().enable();
            if discovery.state.cached.lock().unwrap().refresh.is_none() {
                return;
            }
            done.await;
        }
    }

    #[tokio::test]
    async fn initial_resolution_failure_is_unavailable() {
        let discovery = scripted(vec![Err("DNS unavailable")]);

        assert!(
            discovery
                .backends()
                .await
                .unwrap_err()
                .to_string()
                .contains("DNS unavailable")
        );
    }

    #[tokio::test]
    async fn failed_refresh_uses_only_a_bounded_stale_snapshot() {
        let address = "127.0.0.1:8080".parse().unwrap();
        let discovery = scripted(vec![Ok(vec![address]), Err("transient"), Err("still down")]);
        assert_eq!(
            discovery.backends().await.unwrap().freshness,
            DiscoveryFreshness::Fresh
        );

        age_snapshot(&discovery, Duration::from_secs(11)).await;
        let stale = discovery.backends().await.unwrap();
        assert_eq!(stale.freshness, DiscoveryFreshness::Stale);
        assert_eq!(stale.backends[0].address, address);

        wait_for_refresh(&discovery).await;
        age_snapshot(&discovery, Duration::from_secs(31)).await;
        let error = discovery.backends().await.unwrap_err().to_string();
        assert!(error.contains("expired"));
        assert!(error.contains("still down"));
        let repeated_error = tokio::time::timeout(Duration::from_millis(100), discovery.backends())
            .await
            .expect("a completed refresh failure remains rate limited")
            .unwrap_err();
        assert!(repeated_error.to_string().contains("still down"));
    }

    #[tokio::test]
    async fn expired_snapshot_waits_for_healthy_refresh() {
        let first = "127.0.0.1:8080".parse().unwrap();
        let second = "127.0.0.2:8080".parse().unwrap();
        let discovery = scripted(vec![Ok(vec![first]), Ok(vec![second])]);
        discovery.backends().await.unwrap();
        age_snapshot(&discovery, Duration::from_secs(31)).await;

        let refreshed = discovery.backends().await.unwrap();

        assert_eq!(refreshed.freshness, DiscoveryFreshness::Fresh);
        assert_eq!(refreshed.backends[0].address, second);
    }

    #[tokio::test]
    async fn fresh_cache_skips_resolution_and_success_replaces_topology() {
        let first = "127.0.0.1:8080".parse().unwrap();
        let second = "127.0.0.2:8080".parse().unwrap();
        let discovery = scripted(vec![Ok(vec![first]), Ok(vec![second])]);

        assert_eq!(
            discovery.backends().await.unwrap().backends[0].address,
            first
        );
        assert_eq!(
            discovery.backends().await.unwrap().backends[0].address,
            first
        );
        age_snapshot(&discovery, Duration::from_secs(11)).await;
        assert_eq!(
            discovery.backends().await.unwrap().backends[0].address,
            first
        );
        wait_for_refresh(&discovery).await;
        assert_eq!(
            discovery.backends().await.unwrap().backends[0].address,
            second
        );
    }

    #[tokio::test]
    async fn stale_failures_are_rate_limited_by_the_refresh_interval() {
        let address = "127.0.0.1:8080".parse().unwrap();
        let discovery = scripted(vec![Ok(vec![address]), Err("transient")]);
        discovery.backends().await.unwrap();
        age_snapshot(&discovery, Duration::from_secs(11)).await;
        assert_eq!(
            discovery.backends().await.unwrap().freshness,
            DiscoveryFreshness::Stale
        );
        wait_for_refresh(&discovery).await;
        assert_eq!(
            discovery.backends().await.unwrap().freshness,
            DiscoveryFreshness::Stale
        );
    }

    #[tokio::test]
    async fn cached_readers_do_not_wait_for_one_delayed_refresh() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls = Arc::clone(&calls);
        let resolver = Arc::new(move |_authority: Arc<str>| {
            let call = resolver_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if call > 0 {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Ok(vec![
                    if call == 0 {
                        "127.0.0.1:8080"
                    } else {
                        "127.0.0.2:8080"
                    }
                    .parse()
                    .unwrap(),
                ])
            }) as ResolveFuture
        });
        let discovery = BackendDiscovery::with_resolver(&config(), resolver);
        discovery.backends().await.unwrap();
        age_snapshot(&discovery, Duration::from_secs(11)).await;
        let start = Instant::now();
        let mut readers = tokio::task::JoinSet::new();
        for _ in 0..20 {
            let discovery = discovery.clone();
            readers.spawn(async move { discovery.backends().await.unwrap() });
        }
        while let Some(result) = readers.join_next().await {
            let result = result.unwrap();
            assert_eq!(result.freshness, DiscoveryFreshness::Stale);
            assert_eq!(
                result.backends[0].address,
                "127.0.0.1:8080".parse().unwrap()
            );
        }
        assert!(start.elapsed() < Duration::from_millis(100));
        eprintln!(
            "20 cached readers: {:?}; resolver delay: 250ms",
            start.elapsed()
        );
        wait_for_refresh(&discovery).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            discovery.backends().await.unwrap().backends[0].address,
            "127.0.0.2:8080".parse().unwrap()
        );
    }

    #[tokio::test]
    async fn initial_resolver_timeout_is_unavailable_and_rate_limited() {
        let resolver =
            Arc::new(move |_authority: Arc<str>| Box::pin(std::future::pending()) as ResolveFuture);
        let discovery = BackendDiscovery::with_resolver(&config(), resolver);
        let error = tokio::time::timeout(
            RESOLVE_TIMEOUT + Duration::from_secs(1),
            discovery.backends(),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), discovery.backends())
                .await
                .unwrap()
                .is_err()
        );
    }

    #[tokio::test]
    async fn dropping_discovery_cancels_its_pending_refresh() {
        let started = Arc::new(Notify::new());
        let resolver_started = Arc::clone(&started);
        let (dropped, receiver) = tokio::sync::oneshot::channel::<()>();
        let dropped = Arc::new(Mutex::new(Some(dropped)));
        let resolver = Arc::new(move |_authority: Arc<str>| {
            let started = Arc::clone(&resolver_started);
            let signal = dropped.lock().unwrap().take().unwrap();
            Box::pin(async move {
                let _signal = signal;
                started.notify_one();
                std::future::pending().await
            }) as ResolveFuture
        });
        let discovery = BackendDiscovery::with_resolver(&config(), resolver);
        {
            let mut cached = discovery.state.cached.lock().unwrap();
            cached.snapshot = Some(Snapshot {
                resolved_at: Instant::now() - Duration::from_secs(11),
                backends: normalize(vec!["127.0.0.1:8080".parse().unwrap()]),
            });
        }
        discovery.backends().await.unwrap();
        started.notified().await;
        drop(discovery);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), receiver)
                .await
                .unwrap()
                .is_err()
        );
    }

    #[test]
    fn resolved_addresses_are_sorted_and_deduplicated() {
        let first = "[::2]:8080".parse().unwrap();
        let second = "[::1]:8080".parse().unwrap();
        assert_eq!(
            normalize(vec![first, second, first]),
            normalize(vec![second, first])
        );
        assert_eq!(normalize(vec![first, second, first]).len(), 2);
    }

    #[test]
    fn empty_resolution_is_an_error() {
        assert!(normalize_resolved(Vec::new()).is_err());
    }

    #[test]
    fn ipv6_is_the_single_identity_for_dual_stack_replicas() {
        let ipv6 = "[::1]:8080".parse().unwrap();
        let ipv4 = "127.0.0.1:8080".parse().unwrap();

        assert_eq!(
            normalize(vec![ipv4, ipv6]),
            vec![Backend {
                identity: ipv6.to_string(),
                address: ipv6,
            }]
        );
    }

    #[test]
    fn ipv4_only_environments_still_work() {
        let address = "127.0.0.1:8080".parse().unwrap();

        assert_eq!(normalize(vec![address]).len(), 1);
    }
}
