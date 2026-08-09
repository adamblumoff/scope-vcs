use crate::error::DomainError;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

pub const MAX_RUNNER_NAME_BYTES: usize = 64;
pub const MAX_RUNNER_VERSION_BYTES: usize = 100;
pub const MAX_RUNNER_CONCURRENT_JOBS: u8 = 16;
pub const RUNNER_PROTOCOL_VERSION: u32 = 7;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RunnerMaxConcurrentJobs(u8);

impl RunnerMaxConcurrentJobs {
    pub fn new(value: u8) -> Result<Self, DomainError> {
        if !(1..=MAX_RUNNER_CONCURRENT_JOBS).contains(&value) {
            return Err(DomainError::invalid_input(format!(
                "runner max concurrent jobs must be between 1 and {MAX_RUNNER_CONCURRENT_JOBS}"
            )));
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for RunnerMaxConcurrentJobs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u8::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RunnerName(String);

impl RunnerName {
    pub fn parse(name: impl Into<String>) -> Result<Self, DomainError> {
        let name = name.into();
        if name == "any"
            || name.is_empty()
            || name.len() > MAX_RUNNER_NAME_BYTES
            || !name
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(DomainError::invalid_input(
                "runner name must start with a letter or number and contain only letters, numbers, -, _, or .",
            ));
        }
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerOperatingSystem {
    Linux,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerArchitecture {
    Amd64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerContainerEngine {
    Docker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerCapabilities {
    pub operating_system: RunnerOperatingSystem,
    pub architecture: RunnerArchitecture,
    pub container_engine: RunnerContainerEngine,
}

impl RunnerCapabilities {
    pub fn v1() -> Self {
        Self {
            operating_system: RunnerOperatingSystem::Linux,
            architecture: RunnerArchitecture::Amd64,
            container_engine: RunnerContainerEngine::Docker,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runner {
    pub id: String,
    pub owner_user_id: String,
    pub secret_hash: String,
    pub version: String,
    pub protocol_version: u32,
    pub capabilities: RunnerCapabilities,
    pub max_concurrent_jobs: RunnerMaxConcurrentJobs,
    pub enabled: bool,
    pub created_at_unix: u64,
    pub last_seen_at_unix: Option<u64>,
}

impl Runner {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        owner_user_id: impl Into<String>,
        secret_hash: impl Into<String>,
        version: impl Into<String>,
        protocol_version: u32,
        capabilities: RunnerCapabilities,
        max_concurrent_jobs: RunnerMaxConcurrentJobs,
        created_at_unix: u64,
    ) -> Result<Self, DomainError> {
        let id = required("runner id", id.into())?;
        let owner_user_id = required("runner owner user id", owner_user_id.into())?;
        let secret_hash = secret_hash.into();
        validate_sha256_hash("runner secret hash", &secret_hash)?;
        let version = version.into();
        if version.trim().is_empty() || version.len() > MAX_RUNNER_VERSION_BYTES {
            return Err(DomainError::invalid_input(
                "runner version must contain between 1 and 100 bytes",
            ));
        }
        Ok(Self {
            id,
            owner_user_id,
            secret_hash,
            version,
            protocol_version,
            capabilities,
            max_concurrent_jobs,
            enabled: true,
            created_at_unix,
            last_seen_at_unix: None,
        })
    }

    pub fn supports_dispatch(&self) -> bool {
        self.enabled
            && self.protocol_version == RUNNER_PROTOCOL_VERSION
            && self.capabilities == RunnerCapabilities::v1()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: impl Into<String>,
        owner_user_id: impl Into<String>,
        secret_hash: impl Into<String>,
        version: impl Into<String>,
        protocol_version: u32,
        capabilities: RunnerCapabilities,
        max_concurrent_jobs: RunnerMaxConcurrentJobs,
        enabled: bool,
        created_at_unix: u64,
        last_seen_at_unix: Option<u64>,
    ) -> Result<Self, DomainError> {
        let mut runner = Self::new(
            id,
            owner_user_id,
            secret_hash,
            version,
            protocol_version,
            capabilities,
            max_concurrent_jobs,
            created_at_unix,
        )?;
        runner.enabled = enabled;
        if let Some(last_seen_at_unix) = last_seen_at_unix {
            runner.record_seen(last_seen_at_unix)?;
        }
        Ok(runner)
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn record_seen(&mut self, now_unix: u64) -> Result<(), DomainError> {
        if now_unix < self.created_at_unix {
            return Err(DomainError::invalid_input(
                "runner last-seen time cannot predate registration",
            ));
        }
        self.last_seen_at_unix = Some(
            self.last_seen_at_unix
                .map_or(now_unix, |last_seen| last_seen.max(now_unix)),
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerGrant {
    pub repository_id: String,
    pub runner_id: String,
    pub name: RunnerName,
    pub granted_by_user_id: String,
    pub created_at_unix: u64,
    pub revoked_at_unix: Option<u64>,
}

impl RunnerGrant {
    pub fn new(
        repository_id: impl Into<String>,
        runner_id: impl Into<String>,
        name: RunnerName,
        granted_by_user_id: impl Into<String>,
        created_at_unix: u64,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            repository_id: required("runner grant repository id", repository_id.into())?,
            runner_id: required("runner grant runner id", runner_id.into())?,
            name,
            granted_by_user_id: required("runner grant actor user id", granted_by_user_id.into())?,
            created_at_unix,
            revoked_at_unix: None,
        })
    }

    pub fn is_active(&self) -> bool {
        self.revoked_at_unix.is_none()
    }

    pub fn restore(
        repository_id: impl Into<String>,
        runner_id: impl Into<String>,
        name: RunnerName,
        granted_by_user_id: impl Into<String>,
        created_at_unix: u64,
        revoked_at_unix: Option<u64>,
    ) -> Result<Self, DomainError> {
        let mut grant = Self::new(
            repository_id,
            runner_id,
            name,
            granted_by_user_id,
            created_at_unix,
        )?;
        if let Some(revoked_at_unix) = revoked_at_unix {
            grant.revoke(revoked_at_unix)?;
        }
        Ok(grant)
    }

    pub fn revoke(&mut self, now_unix: u64) -> Result<bool, DomainError> {
        if now_unix < self.created_at_unix {
            return Err(DomainError::invalid_input(
                "runner grant revocation cannot predate the grant",
            ));
        }
        if self.revoked_at_unix.is_some() {
            return Ok(false);
        }
        self.revoked_at_unix = Some(now_unix);
        Ok(true)
    }
}

pub(crate) fn validate_sha256_hash(label: &str, value: &str) -> Result<(), DomainError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DomainError::invalid_input(format!(
            "{label} must be a SHA-256 hex digest"
        )));
    }
    Ok(())
}

pub(crate) fn required(label: &str, value: String) -> Result<String, DomainError> {
    if value.trim().is_empty() {
        Err(DomainError::invalid_input(format!("{label} is required")))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_names_and_protocol_compatibility_are_exact() {
        assert_eq!(
            RunnerName::parse("linux-box").unwrap().as_str(),
            "linux-box"
        );
        for invalid in ["", "any", "-linux", "linux box"] {
            assert!(RunnerName::parse(invalid).is_err(), "{invalid}");
        }

        let runner = Runner::new(
            "runner-1",
            "user-1",
            "a".repeat(64),
            "1.0.0",
            RUNNER_PROTOCOL_VERSION,
            RunnerCapabilities::v1(),
            RunnerMaxConcurrentJobs::new(1).unwrap(),
            10,
        )
        .unwrap();
        assert!(runner.supports_dispatch());
        let previous_protocol = Runner::new(
            "runner-old",
            "user-1",
            "b".repeat(64),
            "0.1.0",
            RUNNER_PROTOCOL_VERSION - 1,
            RunnerCapabilities::v1(),
            RunnerMaxConcurrentJobs::new(1).unwrap(),
            10,
        )
        .unwrap();
        assert!(!previous_protocol.supports_dispatch());
        let future_protocol = Runner::new(
            "runner-future",
            "user-1",
            "c".repeat(64),
            "2.0.0",
            RUNNER_PROTOCOL_VERSION + 1,
            RunnerCapabilities::v1(),
            RunnerMaxConcurrentJobs::new(1).unwrap(),
            10,
        )
        .unwrap();
        assert!(!future_protocol.supports_dispatch());

        let mut reordered = runner;
        reordered.record_seen(20).unwrap();
        reordered.record_seen(19).unwrap();
        assert_eq!(reordered.last_seen_at_unix, Some(20));
        assert!(reordered.record_seen(9).is_err());
    }

    #[test]
    fn runner_capacity_is_strict_in_construction_and_deserialization() {
        assert_eq!(RunnerMaxConcurrentJobs::new(1).unwrap().get(), 1);
        assert_eq!(RunnerMaxConcurrentJobs::new(16).unwrap().get(), 16);
        assert!(RunnerMaxConcurrentJobs::new(0).is_err());
        assert!(RunnerMaxConcurrentJobs::new(17).is_err());
        assert!(serde_json::from_str::<RunnerMaxConcurrentJobs>("0").is_err());
        assert_eq!(
            serde_json::from_str::<RunnerMaxConcurrentJobs>("4")
                .unwrap()
                .get(),
            4
        );
    }
}
