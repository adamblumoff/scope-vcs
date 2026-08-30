use crate::RouterConfig;
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::lookup_host, sync::RwLock};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Backend {
    pub(crate) identity: String,
    pub(crate) address: SocketAddr,
}

#[derive(Clone)]
pub struct BackendDiscovery {
    authority: Arc<str>,
    refresh_after: Duration,
    snapshot: Arc<RwLock<Option<Snapshot>>>,
}

#[derive(Clone)]
struct Snapshot {
    resolved_at: Instant,
    backends: Vec<Backend>,
}

impl BackendDiscovery {
    pub fn new(config: &RouterConfig) -> Self {
        Self {
            authority: Arc::from(config.backend_authority.as_str()),
            refresh_after: config.dns_refresh,
            snapshot: Arc::new(RwLock::new(None)),
        }
    }

    pub(crate) async fn backends(&self) -> anyhow::Result<Vec<Backend>> {
        if let Some(backends) = self.fresh_backends().await {
            return Ok(backends);
        }

        let mut snapshot = self.snapshot.write().await;
        if let Some(current) = snapshot.as_ref()
            && current.resolved_at.elapsed() < self.refresh_after
        {
            return Ok(current.backends.clone());
        }

        match resolve(&self.authority).await {
            Ok(backends) => {
                *snapshot = Some(Snapshot {
                    resolved_at: Instant::now(),
                    backends: backends.clone(),
                });
                Ok(backends)
            }
            Err(error) if snapshot.is_some() => {
                tracing::warn!(%error, authority = %self.authority, "using stale API replica discovery");
                Ok(snapshot
                    .as_ref()
                    .expect("checked stale discovery snapshot")
                    .backends
                    .clone())
            }
            Err(error) => Err(error),
        }
    }

    async fn fresh_backends(&self) -> Option<Vec<Backend>> {
        self.snapshot
            .read()
            .await
            .as_ref()
            .filter(|snapshot| snapshot.resolved_at.elapsed() < self.refresh_after)
            .map(|snapshot| snapshot.backends.clone())
    }

    #[cfg(test)]
    pub(crate) fn fixed(backends: Vec<SocketAddr>) -> Self {
        Self {
            authority: Arc::from("fixed.invalid:8080"),
            refresh_after: Duration::from_secs(3_600),
            snapshot: Arc::new(RwLock::new(Some(Snapshot {
                resolved_at: Instant::now(),
                backends: normalize(backends),
            }))),
        }
    }
}

async fn resolve(authority: &str) -> anyhow::Result<Vec<Backend>> {
    let addresses = lookup_host(authority).await?.collect::<Vec<_>>();
    let backends = normalize(addresses);
    if backends.is_empty() {
        anyhow::bail!("{authority} did not resolve to an API replica");
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
