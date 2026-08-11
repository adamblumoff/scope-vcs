use thiserror::Error;

const REPO_IMAGE_CONTEXT_PREFIX: &str = "/.scope/images/";
const MAX_REPO_IMAGE_NAME_BYTES: usize = 64;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ImageContextPathError {
    #[error(
        "image context path must be /.scope/images/<kebab-name>/<context-path> with an image name no longer than {MAX_REPO_IMAGE_NAME_BYTES} bytes"
    )]
    InvalidPath,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageContextPath(String);

impl ImageContextPath {
    pub fn parse(path: impl Into<String>) -> Result<Self, ImageContextPathError> {
        let path = path.into();
        let Some(relative) = path.strip_prefix(REPO_IMAGE_CONTEXT_PREFIX) else {
            return Err(ImageContextPathError::InvalidPath);
        };
        let Some((image_name, context_path)) = relative.split_once('/') else {
            return Err(ImageContextPathError::InvalidPath);
        };
        if !is_kebab_name(image_name)
            || context_path.is_empty()
            || context_path
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | ".."))
        {
            return Err(ImageContextPathError::InvalidPath);
        }
        Ok(Self(path))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn image_name(&self) -> &str {
        self.relative_parts().0
    }

    pub fn context_path(&self) -> &str {
        self.relative_parts().1
    }

    fn relative_parts(&self) -> (&str, &str) {
        self.0
            .strip_prefix(REPO_IMAGE_CONTEXT_PREFIX)
            .and_then(|relative| relative.split_once('/'))
            .expect("validated image context paths contain an image name and context path")
    }
}

fn is_kebab_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_REPO_IMAGE_NAME_BYTES
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_image_context_files() {
        for (path, image_name, context_path) in [
            ("/.scope/images/checks/Dockerfile", "checks", "Dockerfile"),
            (
                "/.scope/images/checks/.dockerignore",
                "checks",
                ".dockerignore",
            ),
            (
                "/.scope/images/checks/scripts/install.sh",
                "checks",
                "scripts/install.sh",
            ),
            (
                "/.scope/images/e2e-2/fixtures/config.json",
                "e2e-2",
                "fixtures/config.json",
            ),
        ] {
            let parsed = ImageContextPath::parse(path).unwrap();
            assert_eq!(parsed.as_str(), path);
            assert_eq!(parsed.image_name(), image_name);
            assert_eq!(parsed.context_path(), context_path);
        }
    }

    #[test]
    fn rejects_malformed_image_context_paths() {
        let overlong_name = "a".repeat(MAX_REPO_IMAGE_NAME_BYTES + 1);
        for path in [
            "/.scope/images",
            "/.scope/images/",
            "/.scope/images/Dockerfile",
            "/.scope/images/Checks/Dockerfile",
            "/.scope/images/-checks/Dockerfile",
            "/.scope/images/checks-/Dockerfile",
            "/.scope/images/checks--api/Dockerfile",
            "/.scope/images/checks/",
            "/.scope/images/checks//Dockerfile",
            "/.scope/images/checks/../Dockerfile",
        ] {
            assert_eq!(
                ImageContextPath::parse(path),
                Err(ImageContextPathError::InvalidPath),
                "{path}"
            );
        }
        assert_eq!(
            ImageContextPath::parse(format!(
                "{REPO_IMAGE_CONTEXT_PREFIX}{overlong_name}/Dockerfile"
            )),
            Err(ImageContextPathError::InvalidPath)
        );
    }
}
