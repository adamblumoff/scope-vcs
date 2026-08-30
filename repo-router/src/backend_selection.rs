use axum::http::{Method, Uri};
use std::sync::atomic::{AtomicUsize, Ordering};

const UPLOAD_PACK: &str = "git-upload-pack";
const UPLOAD_PACK_PATH_SUFFIX: &str = "/git-upload-pack";
const RECEIVE_PACK_PATH_SUFFIX: &str = "/git-receive-pack";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GitRequestKind {
    UploadPackRead,
    PrimaryOnly,
}

impl GitRequestKind {
    pub(crate) fn classify(method: &Method, uri: &Uri) -> Self {
        match *method {
            Method::POST if uri.path().ends_with(UPLOAD_PACK_PATH_SUFFIX) => Self::UploadPackRead,
            Method::POST if uri.path().ends_with(RECEIVE_PACK_PATH_SUFFIX) => Self::PrimaryOnly,
            Method::GET
                if uri.path().ends_with("/info/refs") && has_one_service(uri, UPLOAD_PACK) =>
            {
                Self::UploadPackRead
            }
            _ => Self::PrimaryOnly,
        }
    }
}

fn has_one_service(uri: &Uri, expected: &str) -> bool {
    let mut services = uri.query().into_iter().flat_map(|query| {
        query.split('&').filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            (name == "service").then_some(value)
        })
    });
    matches!(services.next(), Some(value) if value == expected) && services.next().is_none()
}

pub(crate) struct BackendSelector {
    read_replicas: usize,
    next_read: AtomicUsize,
}

impl BackendSelector {
    pub(crate) fn new(read_replicas: usize) -> Self {
        assert!(read_replicas > 0, "read replica count must be positive");
        Self {
            read_replicas,
            next_read: AtomicUsize::new(0),
        }
    }

    pub(crate) fn select_index(&self, kind: GitRequestKind, backend_count: usize) -> Option<usize> {
        if backend_count == 0 {
            return None;
        }
        if kind == GitRequestKind::PrimaryOnly {
            return Some(0);
        }

        let eligible = self.read_replicas.min(backend_count);
        Some(self.next_read.fetch_add(1, Ordering::Relaxed) % eligible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(value: &str) -> Uri {
        value.parse().unwrap()
    }

    #[test]
    fn classifies_only_upload_pack_operations_as_reads() {
        for (method, uri) in [
            (
                Method::GET,
                uri("/git/public/scope/router/info/refs?service=git-upload-pack"),
            ),
            (
                Method::GET,
                uri("/git/public/scope/router/info/refs?version=2&service=git-upload-pack"),
            ),
            (
                Method::POST,
                uri("/git/permissioned/scope/router/git-upload-pack"),
            ),
        ] {
            assert_eq!(
                GitRequestKind::classify(&method, &uri),
                GitRequestKind::UploadPackRead
            );
        }

        for (method, uri) in [
            (
                Method::GET,
                uri("/git/permissioned/scope/router/info/refs?service=git-receive-pack"),
            ),
            (
                Method::POST,
                uri("/git/permissioned/scope/router/git-receive-pack"),
            ),
            (
                Method::GET,
                uri(
                    "/git/public/scope/router/info/refs?service=git-upload-pack&service=git-receive-pack",
                ),
            ),
            (Method::GET, uri("/git/public/scope/router/info/refs")),
        ] {
            assert_eq!(
                GitRequestKind::classify(&method, &uri),
                GitRequestKind::PrimaryOnly
            );
        }
    }

    #[test]
    fn round_robins_reads_across_the_configured_ranked_prefix() {
        let selector = BackendSelector::new(2);

        assert_eq!(
            (0..6)
                .map(|_| selector
                    .select_index(GitRequestKind::UploadPackRead, 3)
                    .unwrap())
                .collect::<Vec<_>>(),
            vec![0, 1, 0, 1, 0, 1]
        );
    }

    #[test]
    fn pins_primary_operations_and_clamps_reads_to_available_backends() {
        let selector = BackendSelector::new(3);

        assert_eq!(
            selector.select_index(GitRequestKind::PrimaryOnly, 2),
            Some(0)
        );
        assert_eq!(
            (0..4)
                .map(|_| selector
                    .select_index(GitRequestKind::UploadPackRead, 2)
                    .unwrap())
                .collect::<Vec<_>>(),
            vec![0, 1, 0, 1]
        );
        assert_eq!(
            selector.select_index(GitRequestKind::UploadPackRead, 0),
            None
        );
    }
}
