use crate::projection::{LogicalCommitOrigin, NativePublicCommit};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestMergeOrigin {
    Private {
        request_id: String,
        request_head_oid: String,
    },
    Public {
        request_id: String,
        public_base_oid: String,
        public_parent_oids: Vec<String>,
        request_head_oid: String,
        commits: Vec<NativePublicCommit>,
    },
}

impl RequestMergeOrigin {
    pub fn into_logical_origin(self) -> LogicalCommitOrigin {
        match self {
            Self::Private {
                request_id,
                request_head_oid,
            } => LogicalCommitOrigin::PrivateRequestMerge {
                request_id,
                request_head_oid,
            },
            Self::Public {
                request_id,
                public_base_oid,
                public_parent_oids,
                request_head_oid,
                commits,
            } => LogicalCommitOrigin::PublicRequestMerge {
                request_id,
                public_base_oid,
                public_parent_oids,
                request_head_oid,
                commits,
                preserve_public_commits: true,
            },
        }
    }
}
