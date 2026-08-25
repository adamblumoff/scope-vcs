use crate::error::DomainError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedContainerImage(String);

impl PinnedContainerImage {
    pub fn parse(image: impl Into<String>) -> Result<Self, DomainError> {
        let image = image.into();
        let Some((repository, digest)) = image.rsplit_once("@sha256:") else {
            return Err(DomainError::invalid_input(
                "pinned container image must end in an immutable sha256 digest",
            ));
        };
        if repository.is_empty()
            || repository.contains('@')
            || image.chars().any(char::is_whitespace)
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(DomainError::invalid_input(
                "pinned container image is invalid",
            ));
        }
        Ok(Self(format!(
            "{repository}@sha256:{}",
            digest.to_ascii_lowercase()
        )))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn digest(&self) -> &str {
        self.0
            .rsplit_once("@sha256:")
            .map(|(_, digest)| digest)
            .expect("validated pinned images always contain a digest")
    }
}
