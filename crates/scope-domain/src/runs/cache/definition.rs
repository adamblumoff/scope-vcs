use serde::Serialize;
use std::path::{Component, Path};
use thiserror::Error;

pub const MAX_WORKFLOW_CACHE_NAME_BYTES: usize = 64;
pub const MAX_WORKFLOW_CACHE_PATH_BYTES: usize = 1024;
pub const MAX_WORKFLOW_CACHE_FORMAT_BYTES: usize = 64;
pub const MAX_WORKFLOW_CACHE_KEY_INPUTS: usize = 128;
pub const MAX_WORKFLOW_CACHE_INPUT_PATH_BYTES: usize = 1024;
const RESERVED_CACHE_NAME_PREFIX: &str = "scope-";
const RESERVED_CACHE_PATHS: &[&str] = &[
    "/scope-steps",
    "/scope-step.log",
    "/scope-active-step",
    "/workspace/target",
];

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CacheError {
    #[error(
        "workflow cache name must contain between 1 and {MAX_WORKFLOW_CACHE_NAME_BYTES} bytes of lowercase letters, numbers, or single hyphens"
    )]
    InvalidName,
    #[error("workflow cache names beginning with {RESERVED_CACHE_NAME_PREFIX:?} are reserved")]
    ReservedName,
    #[error(
        "workflow cache path must be a normalized Docker-mount-safe absolute path between 1 and {MAX_WORKFLOW_CACHE_PATH_BYTES} bytes"
    )]
    InvalidPath,
    #[error(
        "workflow cache format must contain between 1 and {MAX_WORKFLOW_CACHE_FORMAT_BYTES} bytes of lowercase letters, numbers, or single hyphens"
    )]
    InvalidFormat,
    #[error("workflow cache key cannot contain more than {MAX_WORKFLOW_CACHE_KEY_INPUTS} inputs")]
    TooManyKeyInputs,
    #[error(
        "workflow cache input path must be a normalized repository-relative path between 1 and {MAX_WORKFLOW_CACHE_INPUT_PATH_BYTES} bytes"
    )]
    InvalidInputPath,
    #[error("workflow cache environment input must be a valid shell variable name")]
    InvalidEnvironmentInput,
    #[error("workflow cache key input {0:?} is duplicated")]
    DuplicateKeyInput(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct CacheKeyInputs {
    files: Vec<String>,
    environment: Vec<String>,
    source: bool,
}

impl CacheKeyInputs {
    pub fn new(
        mut files: Vec<String>,
        mut environment: Vec<String>,
        source: bool,
    ) -> Result<Self, CacheError> {
        if files
            .len()
            .saturating_add(environment.len())
            .saturating_add(usize::from(source))
            > MAX_WORKFLOW_CACHE_KEY_INPUTS
        {
            return Err(CacheError::TooManyKeyInputs);
        }
        for path in &files {
            let parsed = Path::new(path);
            if path.is_empty()
                || path.len() > MAX_WORKFLOW_CACHE_INPUT_PATH_BYTES
                || parsed.is_absolute()
                || path
                    .bytes()
                    .any(|byte| matches!(byte, b'\0' | b'\r' | b'\n'))
                || path
                    .split('/')
                    .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
                || parsed
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(CacheError::InvalidInputPath);
            }
        }
        for name in &environment {
            let mut bytes = name.bytes();
            if name.is_empty()
                || !bytes
                    .next()
                    .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
                || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err(CacheError::InvalidEnvironmentInput);
            }
        }
        files.sort();
        environment.sort();
        if let Some(duplicate) = files.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(CacheError::DuplicateKeyInput(duplicate[0].clone()));
        }
        if let Some(duplicate) = environment.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(CacheError::DuplicateKeyInput(duplicate[0].clone()));
        }
        Ok(Self {
            files,
            environment,
            source,
        })
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn environment(&self) -> &[String] {
        &self.environment
    }

    pub fn includes_source(&self) -> bool {
        self.source
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct WorkflowCache {
    name: String,
    path: String,
    format: String,
    compatibility: CacheKeyInputs,
    exact: CacheKeyInputs,
}

impl WorkflowCache {
    pub fn new(
        name: impl Into<String>,
        path: impl Into<String>,
        format: impl Into<String>,
        compatibility: CacheKeyInputs,
        exact: CacheKeyInputs,
    ) -> Result<Self, CacheError> {
        let name = name.into();
        validate_cache_name(&name)?;
        let path = path.into();
        let parsed = Path::new(&path);
        if path.is_empty()
            || path.len() > MAX_WORKFLOW_CACHE_PATH_BYTES
            || path == "/"
            || !parsed.is_absolute()
            || path
                .bytes()
                .any(|byte| matches!(byte, b'\0' | b',' | b'"' | b'\r' | b'\n'))
            || path
                .split('/')
                .skip(1)
                .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
            || parsed
                .components()
                .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
            || parsed == Path::new("/workspace")
            || RESERVED_CACHE_PATHS.iter().any(|reserved| {
                let reserved = Path::new(reserved);
                parsed.starts_with(reserved) || reserved.starts_with(parsed)
            })
        {
            return Err(CacheError::InvalidPath);
        }
        let format = format.into();
        if format.is_empty()
            || format.len() > MAX_WORKFLOW_CACHE_FORMAT_BYTES
            || format.starts_with('-')
            || format.ends_with('-')
            || format.contains("--")
            || !format
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(CacheError::InvalidFormat);
        }
        Ok(Self {
            name,
            path,
            format,
            compatibility,
            exact,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.name
    }

    pub fn mount_path(&self) -> &str {
        &self.path
    }

    pub fn format(&self) -> &str {
        &self.format
    }

    pub fn compatibility_inputs(&self) -> &CacheKeyInputs {
        &self.compatibility
    }

    pub fn exact_inputs(&self) -> &CacheKeyInputs {
        &self.exact
    }
}

pub(super) fn validate_cache_name(name: &str) -> Result<(), CacheError> {
    if name.is_empty()
        || name.len() > MAX_WORKFLOW_CACHE_NAME_BYTES
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(CacheError::InvalidName);
    }
    if name.starts_with(RESERVED_CACHE_NAME_PREFIX) {
        return Err(CacheError::ReservedName);
    }
    Ok(())
}
