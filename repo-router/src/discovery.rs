use crate::RouterConfig;
use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::lookup_host, sync::RwLock};

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
    snapshot: Arc<RwLock<Option<Snapshot>>>,
}

#[derive(Clone)]
struct Snapshot {
    resolved_at: Instant,
    last_attempt_at: Instant,
    last_error: Option<Arc<str>>,
    backends: Vec<Backend>,
}

enum CachedDiscovery {
    Serve(DiscoveredBackends),
    Refresh,
    Expired { age: Duration, error: Arc<str> },
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
            snapshot: Arc::new(RwLock::new(None)),
        }
    }

    pub(crate) async fn backends(&self) -> anyhow::Result<DiscoveredBackends> {
        let now = Instant::now();
        match cached_discovery(
            self.snapshot.read().await.as_ref(),
            now,
            self.refresh_after,
            self.max_stale_age,
        ) {
            CachedDiscovery::Serve(backends) => return Ok(backends),
            CachedDiscovery::Expired { age, error } => {
                return Err(expired_error(&self.authority, age, &error));
            }
            CachedDiscovery::Refresh => {}
        }

        let mut snapshot = self.snapshot.write().await;
        let now = Instant::now();
        match cached_discovery(
            snapshot.as_ref(),
            now,
            self.refresh_after,
            self.max_stale_age,
        ) {
            CachedDiscovery::Serve(backends) => return Ok(backends),
            CachedDiscovery::Expired { age, error } => {
                return Err(expired_error(&self.authority, age, &error));
            }
            CachedDiscovery::Refresh => {}
        }

        match (self.resolver)(Arc::clone(&self.authority))
            .await
            .and_then(normalize_resolved)
        {
            Ok(backends) => {
                let topology_changed = snapshot
                    .as_ref()
                    .is_none_or(|current| current.backends != backends);
                let identities = backends
                    .iter()
                    .map(|backend| backend.identity.as_str())
                    .collect::<Vec<_>>();
                *snapshot = Some(Snapshot {
                    resolved_at: now,
                    last_attempt_at: now,
                    last_error: None,
                    backends: backends.clone(),
                });
                tracing::info!(
                    authority = %self.authority,
                    backend_count = backends.len(),
                    ?identities,
                    topology_changed,
                    discovery_state = "fresh",
                    "refreshed API replica discovery"
                );
                Ok(DiscoveredBackends {
                    backends,
                    freshness: DiscoveryFreshness::Fresh,
                    age: Duration::ZERO,
                })
            }
            Err(error) => self.failed_refresh(&mut snapshot, now, error),
        }
    }

    fn failed_refresh(
        &self,
        snapshot: &mut Option<Snapshot>,
        now: Instant,
        error: anyhow::Error,
    ) -> anyhow::Result<DiscoveredBackends> {
        let error_message: Arc<str> = Arc::from(error.to_string());
        let Some(current) = snapshot.as_mut() else {
            tracing::warn!(
                authority = %self.authority,
                error = %error_message,
                discovery_state = "unavailable",
                "initial API replica discovery failed"
            );
            return Err(error);
        };
        current.last_attempt_at = now;
        current.last_error = Some(Arc::clone(&error_message));
        let age = now.saturating_duration_since(current.resolved_at);
        if age <= self.max_stale_age {
            tracing::warn!(
                authority = %self.authority,
                error = %error_message,
                backend_count = current.backends.len(),
                snapshot_age_ms = age.as_millis(),
                max_stale_age_ms = self.max_stale_age.as_millis(),
                discovery_state = "stale",
                "using bounded stale API replica discovery"
            );
            return Ok(DiscoveredBackends {
                backends: current.backends.clone(),
                freshness: DiscoveryFreshness::Stale,
                age,
            });
        }

        tracing::warn!(
            authority = %self.authority,
            error = %error_message,
            snapshot_age_ms = age.as_millis(),
            max_stale_age_ms = self.max_stale_age.as_millis(),
            discovery_state = "expired",
            "API replica discovery snapshot expired"
        );
        Err(expired_error(&self.authority, age, &error_message))
    }

    #[cfg(test)]
    pub(crate) fn fixed(backends: Vec<SocketAddr>) -> Self {
        let now = Instant::now();
        Self {
            authority: Arc::from("fixed.invalid:8080"),
            refresh_after: Duration::from_secs(3_600),
            max_stale_age: Duration::from_secs(7_200),
            resolver: Arc::new(system_resolver),
            snapshot: Arc::new(RwLock::new(Some(Snapshot {
                resolved_at: now,
                last_attempt_at: now,
                last_error: None,
                backends: normalize(backends),
            }))),
        }
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
            snapshot: Arc::new(RwLock::new(Some(Snapshot {
                resolved_at: now - age,
                last_attempt_at: now,
                last_error: last_error.map(Arc::from),
                backends: normalize(backends),
            }))),
        }
    }
}

fn cached_discovery(
    snapshot: Option<&Snapshot>,
    now: Instant,
    refresh_after: Duration,
    max_stale_age: Duration,
) -> CachedDiscovery {
    let Some(snapshot) = snapshot else {
        return CachedDiscovery::Refresh;
    };
    let age = now.saturating_duration_since(snapshot.resolved_at);
    if age < refresh_after {
        return CachedDiscovery::Serve(DiscoveredBackends {
            backends: snapshot.backends.clone(),
            freshness: DiscoveryFreshness::Fresh,
            age,
        });
    }
    if now.saturating_duration_since(snapshot.last_attempt_at) >= refresh_after {
        return CachedDiscovery::Refresh;
    }
    if age <= max_stale_age {
        return CachedDiscovery::Serve(DiscoveredBackends {
            backends: snapshot.backends.clone(),
            freshness: DiscoveryFreshness::Stale,
            age,
        });
    }
    CachedDiscovery::Expired {
        age,
        error: snapshot
            .last_error
            .clone()
            .unwrap_or_else(|| Arc::from("discovery refresh is pending")),
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
        let mut snapshot = discovery.snapshot.write().await;
        let current = snapshot.as_mut().expect("discovery snapshot");
        current.resolved_at = Instant::now() - age;
        current.last_attempt_at = current.resolved_at;
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

        age_snapshot(&discovery, Duration::from_secs(31)).await;
        assert!(
            discovery
                .backends()
                .await
                .unwrap_err()
                .to_string()
                .contains("expired")
        );
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
        assert_eq!(
            discovery.backends().await.unwrap().freshness,
            DiscoveryFreshness::Stale
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
