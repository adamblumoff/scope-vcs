use super::super::settings::CloudExecutionSettings;
use anyhow::{Context as _, bail};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};

const RUNTIME_ENTRYPOINT: &str = "/scope/bin/scope-runner-runtime";
const EPHEMERAL_STORAGE_MB: u32 = 20 * 1024;

#[derive(Clone)]
pub(crate) struct NorthflankClient {
    client: Client,
    settings: CloudExecutionSettings,
}

#[derive(Debug)]
pub(crate) enum StartError {
    Rejected(anyhow::Error),
    Ambiguous(anyhow::Error),
}

impl NorthflankClient {
    pub(crate) fn new(settings: CloudExecutionSettings) -> anyhow::Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .context("build Northflank API client")?;
        Ok(Self { client, settings })
    }

    pub(crate) async fn start(
        &self,
        image: &str,
        attempt_id: &str,
        bootstrap_token: &str,
    ) -> Result<String, StartError> {
        let mut runtime_environment = BTreeMap::new();
        runtime_environment.insert("SCOPE_API_URL", self.settings.api_url.clone());
        runtime_environment.insert("SCOPE_ATTEMPT_ID", attempt_id.to_string());
        runtime_environment.insert("SCOPE_BOOTSTRAP_TOKEN", bootstrap_token.to_string());
        let body = StartJobRequest {
            runtime_environment,
            billing: BillingOverride {
                deployment_plan: self.settings.northflank_deployment_plan.clone(),
            },
            deployment: DeploymentOverride {
                docker: DockerOverride {
                    config_type: "customEntrypoint",
                    custom_entrypoint: RUNTIME_ENTRYPOINT,
                },
                storage: StorageOverride {
                    ephemeral_storage: EphemeralStorage {
                        storage_size: EPHEMERAL_STORAGE_MB,
                    },
                },
                external: ExternalImage {
                    image_path: image,
                    credentials: self.settings.northflank_registry_credentials_id.as_deref(),
                },
            },
        };
        let response = self
            .client
            .post(self.runs_url())
            .bearer_auth(&self.settings.northflank_api_token)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                StartError::Ambiguous(anyhow::Error::new(error).context("start Northflank job"))
            })?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            let error = anyhow::anyhow!(
                "Northflank rejected job start with {status}: {}",
                truncate(&detail)
            );
            return if status.is_client_error() && status != StatusCode::REQUEST_TIMEOUT {
                Err(StartError::Rejected(error))
            } else {
                Err(StartError::Ambiguous(error))
            };
        }
        let envelope: Envelope<StartedRun> = response.json().await.map_err(|error| {
            StartError::Ambiguous(
                anyhow::Error::new(error).context("decode Northflank start response"),
            )
        })?;
        if envelope.data.id.trim().is_empty() {
            return Err(StartError::Ambiguous(anyhow::anyhow!(
                "Northflank returned an empty run id"
            )));
        }
        Ok(envelope.data.id)
    }

    pub(crate) async fn abort(&self, run_id: &str) -> anyhow::Result<()> {
        let response = self
            .client
            .delete(format!("{}/{}", self.runs_url(), run_id))
            .bearer_auth(&self.settings.northflank_api_token)
            .send()
            .await
            .context("abort Northflank job")?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        bail!(
            "Northflank abort returned {}: {}",
            response.status(),
            truncate(&response.text().await.unwrap_or_default())
        )
    }

    fn runs_url(&self) -> String {
        format!(
            "{}/v1/projects/{}/jobs/{}/runs",
            self.settings.northflank_api_url,
            self.settings.northflank_project_id,
            self.settings.northflank_job_id
        )
    }
}

fn truncate(value: &str) -> String {
    value.chars().take(1024).collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartJobRequest<'a> {
    runtime_environment: BTreeMap<&'static str, String>,
    billing: BillingOverride,
    deployment: DeploymentOverride<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BillingOverride {
    deployment_plan: String,
}

#[derive(Serialize)]
struct DeploymentOverride<'a> {
    docker: DockerOverride<'a>,
    storage: StorageOverride,
    external: ExternalImage<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DockerOverride<'a> {
    config_type: &'static str,
    custom_entrypoint: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageOverride {
    ephemeral_storage: EphemeralStorage,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EphemeralStorage {
    storage_size: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalImage<'a> {
    image_path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    credentials: Option<&'a str>,
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Deserialize)]
struct StartedRun {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn northflank_payload_is_minimal_and_digest_preserving() {
        let body = StartJobRequest {
            runtime_environment: BTreeMap::from([
                ("SCOPE_API_URL", "https://scope.test".into()),
                ("SCOPE_ATTEMPT_ID", "attempt_1".into()),
                ("SCOPE_BOOTSTRAP_TOKEN", "secret".into()),
            ]),
            billing: BillingOverride {
                deployment_plan: "nf-compute-400".into(),
            },
            deployment: DeploymentOverride {
                docker: DockerOverride {
                    config_type: "customEntrypoint",
                    custom_entrypoint: RUNTIME_ENTRYPOINT,
                },
                storage: StorageOverride {
                    ephemeral_storage: EphemeralStorage {
                        storage_size: EPHEMERAL_STORAGE_MB,
                    },
                },
                external: ExternalImage {
                    image_path: "ghcr.io/scope/run@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    credentials: Some("scope-ghcr"),
                },
            },
        };
        let value = serde_json::to_value(body).unwrap();
        assert_eq!(
            value["deployment"]["docker"]["customEntrypoint"],
            RUNTIME_ENTRYPOINT
        );
        assert_eq!(
            value["deployment"]["storage"]["ephemeralStorage"]["storageSize"],
            20_480
        );
        assert!(
            value["deployment"]["external"]["imagePath"]
                .as_str()
                .unwrap()
                .contains("@sha256:")
        );
        assert_eq!(value["deployment"]["external"]["credentials"], "scope-ghcr");
    }
}
