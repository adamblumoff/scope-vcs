use serde::Deserialize;
use std::sync::OnceLock;

const TARGET_MANIFEST_JSON: &str = include_str!("../distribution/targets.json");

#[derive(Debug, Deserialize)]
struct RawManifest {
    schema_version: u16,
    targets: Vec<DistributionTarget>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DistributionTarget {
    pub triple: String,
    pub artifact: String,
    pub os: String,
    pub arch: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct DistributionManifest {
    targets: Vec<DistributionTarget>,
}

impl DistributionManifest {
    pub fn bundled() -> &'static Self {
        static MANIFEST: OnceLock<DistributionManifest> = OnceLock::new();
        MANIFEST.get_or_init(|| {
            Self::from_json(TARGET_MANIFEST_JSON).expect("bundled distribution manifest is valid")
        })
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let raw: RawManifest = serde_json::from_str(json)?;
        assert_eq!(raw.schema_version, 1, "unsupported distribution manifest");
        Ok(Self {
            targets: raw.targets,
        })
    }

    pub fn targets(&self) -> &[DistributionTarget] {
        &self.targets
    }

    pub fn downloadable_file(&self, requested: &str) -> Option<String> {
        self.targets.iter().find_map(|target| {
            if requested == target.artifact {
                Some(target.artifact.clone())
            } else if requested == target.checksum_artifact() {
                Some(requested.to_string())
            } else {
                None
            }
        })
    }

    pub fn required_downloads(&self) -> impl Iterator<Item = String> + '_ {
        self.targets.iter().flat_map(|target| {
            [
                target.artifact.clone(),
                target.checksum_artifact().to_string(),
            ]
        })
    }
}

impl DistributionTarget {
    pub fn checksum_artifact(&self) -> String {
        format!("{}.sha256", self.artifact)
    }
}

#[cfg(test)]
mod tests {
    use super::DistributionManifest;

    #[test]
    fn bundled_manifest_contains_supported_targets() {
        let manifest = DistributionManifest::bundled();
        let targets: Vec<_> = manifest
            .targets()
            .iter()
            .map(|target| target.triple.as_str())
            .collect();

        assert_eq!(
            targets,
            vec![
                "x86_64-unknown-linux-gnu",
                "aarch64-unknown-linux-gnu",
                "x86_64-apple-darwin",
                "aarch64-apple-darwin",
                "x86_64-pc-windows-msvc",
            ]
        );
    }

    #[test]
    fn downloadable_files_are_manifest_artifacts_and_checksums_only() {
        let manifest = DistributionManifest::bundled();

        assert!(
            manifest
                .downloadable_file("scope-x86_64-unknown-linux-gnu")
                .is_some()
        );
        assert!(
            manifest
                .downloadable_file("scope-x86_64-unknown-linux-gnu.sha256")
                .is_some()
        );
        assert!(manifest.downloadable_file("../scope").is_none());
        assert!(manifest.downloadable_file("scope").is_none());
    }
}
