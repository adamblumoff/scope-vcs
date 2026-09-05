use crate::git::projection_repo::hash_field;
use scope_domain::{repository::RepositoryIncarnation, requests::Request};
use sha1::{Digest, Sha1};

const SEMANTICS_VERSION: &str = "named-request-read-view-v3";

/// Identity of the permitted materialization, after request policy filtering.
/// Principal and request lifecycle facts belong to authorization, not this cache.
pub(super) struct GitReadViewIdentity<'a> {
    incarnation: &'a RepositoryIncarnation,
    primary_head: &'a [u8],
    public_base_head: Option<&'a [u8]>,
    refs: Vec<RequestRefIdentity<'a>>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct RequestRefIdentity<'a> {
    name: &'a str,
    head: &'a str,
    snapshot: Option<&'a str>,
    hidden: bool,
}

impl<'a> GitReadViewIdentity<'a> {
    pub(super) fn from_authorized_output(
        incarnation: &'a RepositoryIncarnation,
        primary_head: &'a [u8],
        public_base_head: Option<&'a [u8]>,
        requests: &'a [Request],
        hidden_request_refs: &[String],
    ) -> Self {
        let mut refs: Vec<_> = requests
            .iter()
            .map(|request| RequestRefIdentity {
                name: &request.name,
                head: &request.head_oid,
                snapshot: request
                    .git_snapshot
                    .as_ref()
                    .map(|blob| blob.sha256.as_str()),
                hidden: hidden_request_refs.contains(&request.name),
            })
            .collect();
        refs.sort_unstable();
        Self {
            incarnation,
            primary_head,
            public_base_head,
            refs,
        }
    }

    pub(super) fn cache_key(&self) -> String {
        let mut hasher = Sha1::new();
        for (tag, value) in [
            (
                b"repository".as_slice(),
                self.incarnation.repository_id().as_bytes(),
            ),
            (b"incarnation", self.incarnation.incarnation_id().as_bytes()),
            (b"semantics", SEMANTICS_VERSION.as_bytes()),
            (b"main", self.primary_head),
        ] {
            hash_field(&mut hasher, tag, value);
        }
        if let Some(head) = self.public_base_head {
            hash_field(&mut hasher, b"public-base", head);
        }
        for request in &self.refs {
            hash_field(&mut hasher, b"name", request.name.as_bytes());
            hash_field(&mut hasher, b"head", request.head.as_bytes());
            if let Some(snapshot) = request.snapshot {
                hash_field(&mut hasher, b"snapshot", snapshot.as_bytes());
            }
            hash_field(&mut hasher, b"hidden", &[u8::from(request.hidden)]);
        }
        hex::encode(hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_materialization_fact_changes_the_key() {
        let incarnation = RepositoryIncarnation::new("repo", "first").unwrap();
        let replacement = RepositoryIncarnation::new("repo", "replacement").unwrap();
        let mut identity = GitReadViewIdentity {
            incarnation: &incarnation,
            primary_head: b"primary",
            public_base_head: None,
            refs: vec![RequestRefIdentity {
                name: "request",
                head: "tip",
                snapshot: None,
                hidden: false,
            }],
        };
        let mut keys = std::collections::HashSet::from([identity.cache_key()]);
        identity.public_base_head = Some(b"public-first");
        assert!(keys.insert(identity.cache_key()));
        identity.public_base_head = Some(b"public-second");
        assert!(keys.insert(identity.cache_key()));
        identity.primary_head = b"primary-second";
        assert!(keys.insert(identity.cache_key()));
        identity.refs[0].hidden = true;
        assert!(keys.insert(identity.cache_key()));
        identity.refs[0].snapshot = Some("snapshot-content");
        assert!(keys.insert(identity.cache_key()));
        identity.refs[0].head = "second-tip";
        assert!(keys.insert(identity.cache_key()));
        identity.refs[0].name = "second-request";
        assert!(keys.insert(identity.cache_key()));
        identity.incarnation = &replacement;
        assert!(keys.insert(identity.cache_key()));
        identity.refs.clear();
        assert!(keys.insert(identity.cache_key()));
    }
}
