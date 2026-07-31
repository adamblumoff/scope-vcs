use super::{ClaimRunResponse, RunnerConfig, command_stdout, command_success};
use crate::api::pin_attempt_container_image;
use anyhow::Context;
use reqwest::blocking::Client;
use std::process::Command;

pub(super) fn resolve_container_image(
    client: &Client,
    config: &RunnerConfig,
    claim: &ClaimRunResponse,
) -> anyhow::Result<String> {
    if let Some(image) = &claim.job.pinned_container_image {
        let present = Command::new("docker")
            .args(["image", "inspect", image])
            .output()
            .context("inspect pinned Docker image")?
            .status
            .success();
        if !present {
            command_success(
                Command::new("docker").args(["pull", image]),
                "pull pinned Docker image",
            )?;
        }
        return Ok(image.clone());
    }

    let requested = claim.job.workflow.container().image();
    command_success(
        Command::new("docker").args(["pull", requested]),
        "pull workflow Docker image",
    )?;
    let repo_digests = command_stdout(
        Command::new("docker").args([
            "image",
            "inspect",
            "--format={{json .RepoDigests}}",
            requested,
        ]),
        "resolve workflow Docker image digest",
    )?;
    let repo_digests: Vec<String> = serde_json::from_str(repo_digests.trim())
        .context("parse Docker image repository digests")?;
    let resolved = select_repo_digest(requested, &repo_digests)?;
    Ok(pin_attempt_container_image(
        client,
        &config.api_url,
        &claim.attempt_token,
        &claim.attempt_id,
        resolved,
    )?
    .image)
}

fn select_repo_digest(requested: &str, repo_digests: &[String]) -> anyhow::Result<String> {
    let requested_repository = requested_repository(requested);
    let normalized_repository = normalize_docker_repository(&requested_repository);
    let mut valid = repo_digests
        .iter()
        .filter(|digest| {
            digest
                .rsplit_once("@sha256:")
                .is_some_and(|(repository, hash)| {
                    (repository == requested_repository || repository == normalized_repository)
                        && hash.len() == 64
                        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    valid.sort();
    valid
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Docker did not report an immutable repository digest"))
}

fn requested_repository(image: &str) -> String {
    let image = image
        .split_once('@')
        .map_or(image, |(repository, _)| repository);
    let last_slash = image.rfind('/');
    match image.rfind(':') {
        Some(tag) if last_slash.is_none_or(|slash| tag > slash) => image[..tag].to_string(),
        _ => image.to_string(),
    }
}

fn normalize_docker_repository(repository: &str) -> String {
    if let Some(path) = repository
        .strip_prefix("docker.io/")
        .or_else(|| repository.strip_prefix("index.docker.io/"))
    {
        return if path.contains('/') {
            format!("docker.io/{path}")
        } else {
            format!("docker.io/library/{path}")
        };
    }
    let first = repository.split('/').next().unwrap_or_default();
    if first.contains('.') || first.contains(':') || first == "localhost" {
        repository.to_string()
    } else if repository.contains('/') {
        format!("docker.io/{repository}")
    } else {
        format!("docker.io/library/{repository}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_selection_is_deterministic_and_requires_a_repository_digest() {
        let second = format!("registry.example/b@sha256:{}", "b".repeat(64));
        let first = format!("registry.example/a@sha256:{}", "a".repeat(64));
        assert_eq!(
            select_repo_digest(
                "registry.example/a:latest",
                &[second, "sha256:not-a-repository".to_string(), first.clone()]
            )
            .unwrap(),
            first
        );
        assert!(select_repo_digest("registry.example/a:latest", &[]).is_err());
        assert_eq!(
            select_repo_digest(
                "alpine:3.20",
                &[format!(
                    "docker.io/library/alpine@sha256:{}",
                    "c".repeat(64)
                )]
            )
            .unwrap(),
            format!("docker.io/library/alpine@sha256:{}", "c".repeat(64))
        );
        assert_eq!(
            select_repo_digest(
                "docker.io/alpine:3.20",
                &[format!(
                    "docker.io/library/alpine@sha256:{}",
                    "d".repeat(64)
                )]
            )
            .unwrap(),
            format!("docker.io/library/alpine@sha256:{}", "d".repeat(64))
        );
        assert_eq!(
            select_repo_digest(
                "index.docker.io/alpine:3.20",
                &[format!(
                    "docker.io/library/alpine@sha256:{}",
                    "e".repeat(64)
                )]
            )
            .unwrap(),
            format!("docker.io/library/alpine@sha256:{}", "e".repeat(64))
        );
    }
}
