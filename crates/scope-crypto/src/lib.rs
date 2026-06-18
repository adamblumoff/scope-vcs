use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("manifest signature is invalid")]
    InvalidSignature,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManifestMixedPolicy {
    SyntheticPublicCommit,
    OmitFromPublic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushManifest {
    pub id: Uuid,
    pub repo_id: String,
    pub principal_id: String,
    pub device_id: String,
    pub commit_graph_hash: String,
    pub changed_paths: Vec<String>,
    pub mixed_policy: ManifestMixedPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedPushManifest {
    pub manifest: PushManifest,
    pub signature_hex: String,
}

impl PushManifest {
    pub fn new(
        repo_id: impl Into<String>,
        principal_id: impl Into<String>,
        device_id: impl Into<String>,
        commit_graph_hash: impl Into<String>,
        changed_paths: Vec<String>,
        mixed_policy: ManifestMixedPolicy,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            repo_id: repo_id.into(),
            principal_id: principal_id.into(),
            device_id: device_id.into(),
            commit_graph_hash: commit_graph_hash.into(),
            changed_paths,
            mixed_policy,
        }
    }
}

pub fn sign_manifest(
    manifest: PushManifest,
    device_secret: &[u8],
) -> Result<SignedPushManifest, ManifestError> {
    let signature_hex = sign_payload(&manifest, device_secret)?;
    Ok(SignedPushManifest {
        manifest,
        signature_hex,
    })
}

pub fn verify_manifest(
    signed: &SignedPushManifest,
    device_secret: &[u8],
) -> Result<(), ManifestError> {
    let expected = sign_payload(&signed.manifest, device_secret)?;
    if expected == signed.signature_hex {
        Ok(())
    } else {
        Err(ManifestError::InvalidSignature)
    }
}

fn sign_payload(manifest: &PushManifest, device_secret: &[u8]) -> Result<String, ManifestError> {
    let payload = serde_json::to_vec(manifest)?;
    let mut mac =
        HmacSha256::new_from_slice(device_secret).expect("HMAC accepts any key length for Sha256");
    mac.update(&payload);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_manifest_round_trips() {
        let manifest = PushManifest::new(
            "scope",
            "owner",
            "device",
            "graph",
            vec!["/README.md".to_string()],
            ManifestMixedPolicy::SyntheticPublicCommit,
        );
        let signed = sign_manifest(manifest, b"secret").unwrap();

        verify_manifest(&signed, b"secret").unwrap();
        assert!(verify_manifest(&signed, b"wrong").is_err());
    }
}
