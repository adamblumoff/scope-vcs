use crate::error::DomainError;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

pub const MIN_JOB_CPU_MILLIS: u64 = 500;
pub const MAX_JOB_CPU_MILLIS: u64 = 64_000;
pub const MIN_JOB_MEMORY_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_JOB_MEMORY_BYTES: u64 = 1024 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct JobResources {
    cpu_millis: u64,
    memory_bytes: u64,
}

impl JobResources {
    pub fn new(cpu_millis: u64, memory_bytes: u64) -> Result<Self, DomainError> {
        if !(MIN_JOB_CPU_MILLIS..=MAX_JOB_CPU_MILLIS).contains(&cpu_millis) {
            return Err(DomainError::invalid_input(format!(
                "job CPU must be between {MIN_JOB_CPU_MILLIS} and {MAX_JOB_CPU_MILLIS} millicpus"
            )));
        }
        if !(MIN_JOB_MEMORY_BYTES..=MAX_JOB_MEMORY_BYTES).contains(&memory_bytes) {
            return Err(DomainError::invalid_input(format!(
                "job memory must be between {MIN_JOB_MEMORY_BYTES} and {MAX_JOB_MEMORY_BYTES} bytes"
            )));
        }
        Ok(Self {
            cpu_millis,
            memory_bytes,
        })
    }

    pub fn cpu_millis(self) -> u64 {
        self.cpu_millis
    }

    pub fn memory_bytes(self) -> u64 {
        self.memory_bytes
    }

    pub fn fits_within(self, available: Self) -> bool {
        self.cpu_millis <= available.cpu_millis && self.memory_bytes <= available.memory_bytes
    }
}

impl<'de> Deserialize<'de> for JobResources {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct PersistedJobResources {
            cpu_millis: u64,
            memory_bytes: u64,
        }

        let resources = PersistedJobResources::deserialize(deserializer)?;
        Self::new(resources.cpu_millis, resources.memory_bytes).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_round_trips_resources() {
        let resources = JobResources::new(3_000, 6 * 1024 * 1024 * 1024).unwrap();
        assert_eq!(resources.cpu_millis(), 3_000);
        assert_eq!(resources.memory_bytes(), 6 * 1024 * 1024 * 1024);
        assert_eq!(
            serde_json::from_value::<JobResources>(serde_json::to_value(resources).unwrap())
                .unwrap(),
            resources
        );
    }

    #[test]
    fn rejects_resources_outside_the_execution_contract() {
        assert!(JobResources::new(MIN_JOB_CPU_MILLIS - 1, MIN_JOB_MEMORY_BYTES).is_err());
        assert!(JobResources::new(MIN_JOB_CPU_MILLIS, MIN_JOB_MEMORY_BYTES - 1).is_err());
        assert!(JobResources::new(MAX_JOB_CPU_MILLIS + 1, MIN_JOB_MEMORY_BYTES).is_err());
        assert!(JobResources::new(MIN_JOB_CPU_MILLIS, MAX_JOB_MEMORY_BYTES + 1).is_err());
    }

    #[test]
    fn fits_only_when_both_dimensions_fit() {
        let request = JobResources::new(2_000, 2 * 1024 * 1024 * 1024).unwrap();
        assert!(request.fits_within(JobResources::new(4_000, 4 * 1024 * 1024 * 1024).unwrap()));
        assert!(!request.fits_within(JobResources::new(1_000, 4 * 1024 * 1024 * 1024).unwrap()));
        assert!(!request.fits_within(JobResources::new(4_000, 1024 * 1024 * 1024).unwrap()));
    }
}
