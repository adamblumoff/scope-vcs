use sha2::{Digest, Sha256};

pub fn rank_backends<'a>(repository: &str, backends: &'a [String]) -> Vec<&'a str> {
    let mut ranked = backends
        .iter()
        .map(|backend| (score(repository, backend), backend.as_str()))
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|left, right| right.cmp(left));
    ranked.into_iter().map(|(_, backend)| backend).collect()
}

fn score(repository: &str, backend: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, repository.as_bytes());
    hash_field(&mut hasher, backend.as_bytes());
    hasher.finalize().into()
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn backends(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn ranking_is_stable_and_independent_of_discovery_order() {
        let first = backends(&["10.0.0.3:8080", "10.0.0.1:8080", "10.0.0.2:8080"]);
        let second = backends(&["10.0.0.2:8080", "10.0.0.3:8080", "10.0.0.1:8080"]);

        assert_eq!(
            rank_backends("scope/router", &first),
            rank_backends("scope/router", &second)
        );
    }

    #[test]
    fn adding_one_backend_only_moves_repositories_to_that_backend() {
        let original = backends(&["api-a:8080", "api-b:8080", "api-c:8080"]);
        let expanded = backends(&["api-a:8080", "api-b:8080", "api-c:8080", "api-d:8080"]);

        for index in 0..10_000 {
            let repository = format!("owner/repo-{index}");
            let before = rank_backends(&repository, &original)[0];
            let after = rank_backends(&repository, &expanded)[0];
            assert!(before == after || after == "api-d:8080");
        }
    }

    #[test]
    fn repositories_spread_across_backends() {
        let nodes = backends(&["api-a:8080", "api-b:8080", "api-c:8080"]);
        let mut counts = BTreeMap::new();
        for index in 0..3_000 {
            let repository = format!("owner/repo-{index}");
            *counts
                .entry(rank_backends(&repository, &nodes)[0])
                .or_insert(0_usize) += 1;
        }

        assert_eq!(counts.len(), nodes.len());
        assert!(counts.values().all(|count| (850..=1_150).contains(count)));
    }
}
