use super::definition::{
    MAX_CONTAINER_IMAGE_BYTES, MAX_ENVIRONMENT_KEY_BYTES, MAX_ENVIRONMENT_VALUE_BYTES,
    MAX_STEP_COMMAND_BYTES, MAX_STEP_NAME_BYTES, MAX_WORKFLOW_CACHES,
    MAX_WORKFLOW_ENVIRONMENT_BYTES, MAX_WORKFLOW_ENVIRONMENT_VARIABLES, MAX_WORKFLOW_JOB_ID_BYTES,
    MAX_WORKFLOW_JOBS, MAX_WORKFLOW_NAME_BYTES, MAX_WORKFLOW_STEPS, MAX_WORKFLOW_TIMEOUT_SECONDS,
};
use crate::runs::cache::definition::CacheError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("workflow repository id is required")]
    MissingRepositoryId,
    #[error("workflow path must be /.scope/runs/<kebab-name>.yml or .yaml")]
    InvalidPath,
    #[error("workflow name must contain between 1 and {MAX_WORKFLOW_NAME_BYTES} bytes")]
    InvalidName,
    #[error("workflow must enable at least one trigger")]
    MissingTrigger,
    #[error(
        "container image must be pinned with an immutable sha256 digest and contain at most {MAX_CONTAINER_IMAGE_BYTES} bytes"
    )]
    InvalidContainerImage,
    #[error("workflow timeout must be between 1 and {MAX_WORKFLOW_TIMEOUT_SECONDS} seconds")]
    InvalidTimeout,
    #[error("workflow must contain at least one job")]
    MissingJobs,
    #[error("workflow cannot contain more than {MAX_WORKFLOW_JOBS} jobs")]
    TooManyJobs,
    #[error(
        "workflow job id must be a lowercase kebab name between 1 and {MAX_WORKFLOW_JOB_ID_BYTES} bytes"
    )]
    InvalidJobId,
    #[error("workflow job id {0:?} is duplicated")]
    DuplicateJobId(String),
    #[error("workflow job {job:?} depends on itself")]
    SelfDependency { job: String },
    #[error("workflow job {job:?} names dependency {dependency:?} more than once")]
    DuplicateDependency { job: String, dependency: String },
    #[error("workflow job {job:?} depends on missing job {dependency:?}")]
    MissingDependency { job: String, dependency: String },
    #[error("workflow job dependencies contain a cycle")]
    DependencyCycle,
    #[error("workflow job must contain at least one step")]
    MissingSteps,
    #[error("workflow job cannot contain more than {MAX_WORKFLOW_STEPS} steps")]
    TooManySteps,
    #[error("workflow job cannot contain more than {MAX_WORKFLOW_CACHES} caches")]
    TooManyCaches,
    #[error("workflow job cache name {0:?} is duplicated")]
    DuplicateCacheName(String),
    #[error("workflow job cache path {0:?} overlaps another cache mount")]
    OverlappingCachePath(String),
    #[error("workflow cache environment input {0:?} is not declared by the workflow job")]
    UndeclaredCacheEnvironmentInput(String),
    #[error(
        "workflow job cannot define more than {MAX_WORKFLOW_ENVIRONMENT_VARIABLES} environment variables"
    )]
    TooManyEnvironmentVariables,
    #[error(
        "workflow environment key must be a shell variable name between 1 and {MAX_ENVIRONMENT_KEY_BYTES} bytes"
    )]
    InvalidEnvironmentKey,
    #[error(
        "workflow environment value cannot exceed {MAX_ENVIRONMENT_VALUE_BYTES} bytes or contain a null byte"
    )]
    InvalidEnvironmentValue,
    #[error(
        "workflow environment cannot exceed {MAX_WORKFLOW_ENVIRONMENT_BYTES} bytes in aggregate"
    )]
    EnvironmentTooLarge,
    #[error("workflow step name must contain between 1 and {MAX_STEP_NAME_BYTES} bytes")]
    InvalidStepName,
    #[error("workflow step name {0:?} is duplicated")]
    DuplicateStepName(String),
    #[error("workflow step command must contain between 1 and {MAX_STEP_COMMAND_BYTES} bytes")]
    InvalidStepCommand,
    #[error("workflow revision digest could not be serialized: {0}")]
    Digest(serde_json::Error),
    #[error(transparent)]
    InvalidCache(#[from] CacheError),
}
