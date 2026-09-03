use super::super::settings::CloudExecutionSettings;
use anyhow::{Context as _, bail};
use aws_config::BehaviorVersion;
#[cfg(test)]
use aws_sdk_ecs::types::{Compatibility, TaskDefinition};
use aws_sdk_ecs::{
    Client as EcsSdkClient,
    config::Region,
    error::ProvideErrorMetadata,
    types::{
        AssignPublicIp, AwsVpcConfiguration, ContainerDefinition, ContainerOverride,
        CpuArchitecture, Failure, KeyValuePair, LaunchType, LogConfiguration, LogDriver,
        NetworkConfiguration, NetworkMode, OsFamily, RepositoryCredentials, RuntimePlatform,
        Secret, SortOrder, Tag, Task, TaskDefinitionStatus, TaskOverride,
    },
};
use aws_sdk_secretsmanager::{Client as SecretsManagerClient, types::Tag as SecretTag};
use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::{Instant, sleep};

const CONTAINER_NAME: &str = "scope-runner";
const BOOTSTRAP_SECRET_ENV: &str = "SCOPE_BOOTSTRAP_TOKEN";
const RUNTIME_ENTRYPOINT: &str = "/scope/bin/scope-runner-runtime";
const TASK_CPU: &str = "8192";
const TASK_MEMORY: &str = "16384";
const TASK_FAMILY_PREFIX: &str = "scope-runner-";
const AMBIGUOUS_START_CONSISTENCY_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CONSISTENCY_INITIAL_DELAY: Duration = Duration::from_secs(2);
const CONSISTENCY_MAX_DELAY: Duration = Duration::from_secs(30);
const STOP_POLL_INTERVAL: Duration = Duration::from_secs(2);
const STOP_WAIT_TIMEOUT: Duration = Duration::from_secs(2 * 60);

#[derive(Clone)]
pub(crate) struct EcsClient {
    client: EcsSdkClient,
    secrets: SecretsManagerClient,
    settings: CloudExecutionSettings,
}

#[derive(Debug)]
pub(crate) enum StartError {
    Rejected(anyhow::Error),
    Ambiguous(anyhow::Error),
}

impl EcsClient {
    pub(crate) async fn new(settings: CloudExecutionSettings) -> Self {
        let config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(settings.aws_region.clone()))
            .load()
            .await;
        Self {
            client: EcsSdkClient::new(&config),
            secrets: SecretsManagerClient::new(&config),
            settings,
        }
    }

    pub(crate) async fn start(
        &self,
        image: &str,
        attempt_id: &str,
        bootstrap_token: &str,
        deadline_unix: u64,
    ) -> Result<String, StartError> {
        let secret_arn = match self
            .create_bootstrap_secret(attempt_id, bootstrap_token)
            .await
        {
            Ok(secret_arn) => secret_arn,
            Err(error) => return Err(self.reject_after_cleanup(attempt_id, error).await),
        };
        let task_definition = match self
            .register_task_definition(attempt_id, image, &secret_arn)
            .await
        {
            Ok(task_definition) => task_definition,
            Err(error) => return Err(self.reject_after_cleanup(attempt_id, error).await),
        };
        let network = AwsVpcConfiguration::builder()
            .assign_public_ip(AssignPublicIp::Enabled)
            .set_subnets(Some(self.settings.ecs_subnet_ids.clone()))
            .security_groups(self.settings.ecs_security_group_id.clone())
            .build()
            .map_err(|error| {
                StartError::Rejected(anyhow::Error::new(error).context("build ECS task network"))
            })?;
        let overrides = task_override(&self.settings.api_url, attempt_id, deadline_unix);
        let result = match self
            .client
            .run_task()
            .cluster(&self.settings.ecs_cluster_arn)
            .task_definition(task_definition)
            .launch_type(LaunchType::Fargate)
            .platform_version("LATEST")
            .count(1)
            .client_token(attempt_id)
            .started_by(attempt_id)
            .enable_ecs_managed_tags(true)
            .network_configuration(
                NetworkConfiguration::builder()
                    .awsvpc_configuration(network)
                    .build(),
            )
            .overrides(overrides)
            .tags(scope_tag("Project", "scope-vcs"))
            .tags(scope_tag("Component", "cloud-runner"))
            .tags(scope_tag("AttemptId", attempt_id))
            .send()
            .await
        {
            Ok(result) => result,
            Err(error) => {
                let rejected = error
                    .as_service_error()
                    .is_some_and(|error| !error.is_server_exception());
                let error = anyhow::Error::new(error).context("run ECS task");
                if rejected {
                    return Err(self.reject_after_cleanup(attempt_id, error).await);
                } else {
                    return Err(StartError::Ambiguous(error));
                }
            }
        };

        if !result.failures().is_empty() {
            let failures = result
                .failures()
                .iter()
                .map(|failure| {
                    format!(
                        "{}: {}",
                        failure.arn().unwrap_or("unknown resource"),
                        failure.reason().unwrap_or("unknown reason")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            return Err(self
                .reject_after_cleanup(
                    attempt_id,
                    anyhow::anyhow!("ECS rejected task start: {failures}"),
                )
                .await);
        }
        result
            .tasks()
            .first()
            .and_then(|task| task.task_arn())
            .filter(|arn| !arn.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                StartError::Ambiguous(anyhow::anyhow!("ECS returned no task ARN and no failure"))
            })
    }

    pub(crate) async fn stop_terminal_task(
        &self,
        attempt_id: &str,
        task_arn: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(task_arn) = task_arn {
            self.stop_task_and_wait(task_arn).await?;
            return self.cleanup_attempt_resources(attempt_id).await;
        }
        for task_arn in self.find_tasks_after_ambiguous_start(attempt_id).await? {
            self.stop_task_and_wait(&task_arn).await?;
        }
        self.cleanup_attempt_resources(attempt_id).await
    }

    async fn create_bootstrap_secret(
        &self,
        attempt_id: &str,
        bootstrap_token: &str,
    ) -> anyhow::Result<String> {
        let result = self
            .secrets
            .create_secret()
            .name(secret_name(
                &self.settings.ecs_cluster_arn,
                attempt_id,
                &self.settings.ecs_secret_name_key,
            )?)
            .client_request_token(attempt_id)
            .secret_string(bootstrap_token)
            .tags(secret_tag("Project", "scope-vcs"))
            .tags(secret_tag("Component", "cloud-runner"))
            .tags(secret_tag("AttemptId", attempt_id))
            .send()
            .await
            .context("create per-attempt bootstrap secret")?;
        result
            .arn()
            .filter(|arn| !arn.is_empty())
            .map(str::to_string)
            .context("Secrets Manager created a bootstrap secret without an ARN")
    }

    async fn reject_after_cleanup(&self, attempt_id: &str, error: anyhow::Error) -> StartError {
        match self.cleanup_attempt_resources(attempt_id).await {
            Ok(()) => StartError::Rejected(error),
            Err(cleanup_error) => StartError::Ambiguous(error.context(format!(
                "dispatch setup failed and its AWS resources could not be reconciled: {cleanup_error:#}"
            ))),
        }
    }

    async fn cleanup_attempt_resources(&self, attempt_id: &str) -> anyhow::Result<()> {
        let family = task_family(attempt_id)?;
        let definitions = self
            .client
            .list_task_definitions()
            .family_prefix(&family)
            .status(TaskDefinitionStatus::Active)
            .sort(SortOrder::Desc)
            .max_results(100)
            .send()
            .await
            .context("list per-attempt ECS task definitions for cleanup")?;
        for arn in definitions
            .task_definition_arns()
            .iter()
            .filter(|arn| task_definition_family(arn) == Some(family.as_str()))
        {
            self.client
                .deregister_task_definition()
                .task_definition(arn)
                .send()
                .await
                .context("deregister per-attempt ECS task definition")?;
        }

        let secret_name = secret_name(
            &self.settings.ecs_cluster_arn,
            attempt_id,
            &self.settings.ecs_secret_name_key,
        )?;
        match self
            .secrets
            .delete_secret()
            .secret_id(secret_name)
            .force_delete_without_recovery(true)
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|error| error.is_resource_not_found_exception()) =>
            {
                Ok(())
            }
            Err(error) => Err(anyhow::Error::new(error).context("delete bootstrap secret")),
        }
    }

    async fn find_tasks_after_ambiguous_start(
        &self,
        attempt_id: &str,
    ) -> anyhow::Result<Vec<String>> {
        let deadline = Instant::now() + AMBIGUOUS_START_CONSISTENCY_TIMEOUT;
        let mut delay = CONSISTENCY_INITIAL_DELAY;
        loop {
            let tasks = self
                .client
                .list_tasks()
                .cluster(&self.settings.ecs_cluster_arn)
                .started_by(attempt_id)
                .send()
                .await
                .context("find ECS task by attempt id")?;
            if !tasks.task_arns().is_empty() {
                return Ok(tasks.task_arns().to_vec());
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(Vec::new());
            }
            sleep(delay.min(deadline.saturating_duration_since(now))).await;
            delay = next_consistency_delay(delay);
        }
    }

    async fn stop_task_and_wait(&self, task_arn: &str) -> anyhow::Result<()> {
        match self
            .client
            .stop_task()
            .cluster(&self.settings.ecs_cluster_arn)
            .task(task_arn)
            .reason("Scope run canceled")
            .send()
            .await
        {
            Ok(_) => self.wait_until_stopped(task_arn).await,
            Err(error) => {
                let missing = error.as_service_error().is_some_and(|service_error| {
                    service_error.is_client_exception()
                        && service_error.message().is_some_and(|message| {
                            message.to_ascii_lowercase().contains("not found")
                        })
                });
                if missing {
                    Ok(())
                } else {
                    Err(anyhow::Error::new(error).context("stop ECS task"))
                }
            }
        }
    }

    async fn wait_until_stopped(&self, task_arn: &str) -> anyhow::Result<()> {
        let deadline = Instant::now() + STOP_WAIT_TIMEOUT;
        loop {
            let described = self
                .client
                .describe_tasks()
                .cluster(&self.settings.ecs_cluster_arn)
                .tasks(task_arn)
                .send()
                .await
                .context("describe stopping ECS task")?;
            if task_has_stopped(described.tasks(), described.failures(), task_arn)? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("ECS task {task_arn} did not reach STOPPED within two minutes");
            }
            sleep(STOP_POLL_INTERVAL).await;
        }
    }

    async fn register_task_definition(
        &self,
        attempt_id: &str,
        image: &str,
        secret_arn: &str,
    ) -> anyhow::Result<String> {
        let family = task_family(attempt_id)?;
        let container = runner_container(
            image,
            &self.settings.aws_region,
            &self.settings.ecs_log_group,
            secret_arn,
            self.settings.registry_credentials_secret_arn.as_deref(),
        )?;
        let result = self
            .client
            .register_task_definition()
            .family(family)
            .network_mode(NetworkMode::Awsvpc)
            .requires_compatibilities(aws_sdk_ecs::types::Compatibility::Fargate)
            .cpu(TASK_CPU)
            .memory(TASK_MEMORY)
            .execution_role_arn(&self.settings.ecs_execution_role_arn)
            .runtime_platform(runtime_platform())
            .container_definitions(container)
            .tags(scope_tag("Project", "scope-vcs"))
            .tags(scope_tag("Component", "cloud-runner"))
            .tags(scope_tag("ImageDigest", image_digest(image)?))
            .send()
            .await
            .context("register ECS task definition")?;
        result
            .task_definition()
            .and_then(|definition| definition.task_definition_arn())
            .filter(|arn| !arn.is_empty())
            .map(str::to_string)
            .context("ECS registered a task definition without an ARN")
    }
}

fn task_has_stopped(tasks: &[Task], failures: &[Failure], task_arn: &str) -> anyhow::Result<bool> {
    if tasks
        .iter()
        .any(|task| task.last_status() == Some("STOPPED"))
        || failures.iter().any(|failure| {
            failure
                .reason()
                .is_some_and(|reason| reason.eq_ignore_ascii_case("MISSING"))
        })
    {
        return Ok(true);
    }
    if !failures.is_empty() {
        let failures = failures
            .iter()
            .map(|failure| {
                format!(
                    "{}: {}",
                    failure.arn().unwrap_or(task_arn),
                    failure.reason().unwrap_or("unknown reason")
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        bail!("ECS rejected task status lookup: {failures}");
    }
    Ok(false)
}

fn next_consistency_delay(delay: Duration) -> Duration {
    delay.saturating_mul(2).min(CONSISTENCY_MAX_DELAY)
}

fn environment(name: &str, value: &str) -> KeyValuePair {
    KeyValuePair::builder().name(name).value(value).build()
}

fn task_override(api_url: &str, attempt_id: &str, deadline_unix: u64) -> TaskOverride {
    let container = ContainerOverride::builder()
        .name(CONTAINER_NAME)
        .set_environment(Some(vec![
            environment("SCOPE_API_URL", api_url),
            environment("SCOPE_ATTEMPT_ID", attempt_id),
            environment("SCOPE_ATTEMPT_DEADLINE_UNIX", &deadline_unix.to_string()),
        ]))
        .build();
    TaskOverride::builder()
        .container_overrides(container)
        .build()
}

fn runtime_platform() -> RuntimePlatform {
    RuntimePlatform::builder()
        .cpu_architecture(CpuArchitecture::X8664)
        .operating_system_family(OsFamily::Linux)
        .build()
}

fn runner_container(
    image: &str,
    region: &str,
    log_group: &str,
    bootstrap_secret_arn: &str,
    registry_credentials_secret_arn: Option<&str>,
) -> anyhow::Result<ContainerDefinition> {
    let log_options = HashMap::from([
        ("awslogs-group".to_string(), log_group.to_string()),
        ("awslogs-region".to_string(), region.to_string()),
        ("awslogs-stream-prefix".to_string(), "runner".to_string()),
    ]);
    let log_configuration = LogConfiguration::builder()
        .log_driver(LogDriver::Awslogs)
        .set_options(Some(log_options))
        .build()
        .context("build ECS log configuration")?;
    let repository_credentials = registry_credentials_secret_arn
        .map(|arn| {
            RepositoryCredentials::builder()
                .credentials_parameter(arn)
                .build()
                .context("build ECS repository credentials")
        })
        .transpose()?;
    Ok(ContainerDefinition::builder()
        .name(CONTAINER_NAME)
        .image(image)
        .essential(true)
        .entry_point(RUNTIME_ENTRYPOINT)
        .set_repository_credentials(repository_credentials)
        .secrets(
            Secret::builder()
                .name(BOOTSTRAP_SECRET_ENV)
                .value_from(bootstrap_secret_arn)
                .build()
                .context("build ECS bootstrap secret reference")?,
        )
        .log_configuration(log_configuration)
        .build())
}

#[cfg(test)]
fn verify_task_definition_contract(
    definition: &TaskDefinition,
    image: &str,
    bootstrap_secret_arn: &str,
    settings: &CloudExecutionSettings,
) -> anyhow::Result<()> {
    if definition.network_mode() != Some(&NetworkMode::Awsvpc)
        || definition.requires_compatibilities() != [Compatibility::Fargate]
        || definition.cpu() != Some(TASK_CPU)
        || definition.memory() != Some(TASK_MEMORY)
        || definition.execution_role_arn() != Some(settings.ecs_execution_role_arn.as_str())
        || definition.task_role_arn().is_some()
        || definition.runtime_platform() != Some(&runtime_platform())
        || definition.ephemeral_storage().is_some()
        || !definition.volumes().is_empty()
        || !definition.placement_constraints().is_empty()
        || !definition.inference_accelerators().is_empty()
        || definition.pid_mode().is_some()
        || definition.ipc_mode().is_some()
        || definition.proxy_configuration().is_some()
        || definition.enable_fault_injection() == Some(true)
        || definition.container_definitions().len() != 1
    {
        bail!("stored definition differs from the expected Fargate task");
    }
    verify_container_contract(
        &definition.container_definitions()[0],
        image,
        &settings.aws_region,
        &settings.ecs_log_group,
        bootstrap_secret_arn,
        settings.registry_credentials_secret_arn.as_deref(),
    )?;
    Ok(())
}

#[cfg(test)]
fn verify_container_contract(
    container: &ContainerDefinition,
    image: &str,
    region: &str,
    log_group: &str,
    bootstrap_secret_arn: &str,
    registry_credentials_secret_arn: Option<&str>,
) -> anyhow::Result<()> {
    let expected_log_options = HashMap::from([
        ("awslogs-group".to_string(), log_group.to_string()),
        ("awslogs-region".to_string(), region.to_string()),
        ("awslogs-stream-prefix".to_string(), "runner".to_string()),
    ]);
    let logs = container
        .log_configuration()
        .context("stored definition is missing the log configuration")?;
    let expected_repository_credentials = registry_credentials_secret_arn
        .map(|arn| {
            RepositoryCredentials::builder()
                .credentials_parameter(arn)
                .build()
                .context("build expected ECS repository credentials")
        })
        .transpose()?;
    if container.name() != Some(CONTAINER_NAME)
        || container.image() != Some(image)
        || container.essential() != Some(true)
        || container.entry_point() != [RUNTIME_ENTRYPOINT]
        || container.cpu() != 0
        || container.memory().is_some()
        || container.memory_reservation().is_some()
        || container.repository_credentials() != expected_repository_credentials.as_ref()
        || !container.links().is_empty()
        || !container.port_mappings().is_empty()
        || container.restart_policy().is_some()
        || !container.command().is_empty()
        || !container.environment().is_empty()
        || !container.environment_files().is_empty()
        || !container.mount_points().is_empty()
        || !container.volumes_from().is_empty()
        || container.linux_parameters().is_some()
        || container.secrets()
            != [Secret::builder()
                .name(BOOTSTRAP_SECRET_ENV)
                .value_from(bootstrap_secret_arn)
                .build()
                .context("build expected ECS bootstrap secret reference")?]
        || !container.depends_on().is_empty()
        || container.start_timeout().is_some()
        || container.stop_timeout().is_some()
        || container.hostname().is_some()
        || container.user().is_some()
        || container.working_directory().is_some()
        || container.disable_networking() == Some(true)
        || container.privileged() == Some(true)
        || container.readonly_root_filesystem() == Some(true)
        || !container.dns_servers().is_empty()
        || !container.dns_search_domains().is_empty()
        || !container.extra_hosts().is_empty()
        || !container.docker_security_options().is_empty()
        || container.interactive() == Some(true)
        || container.pseudo_terminal() == Some(true)
        || container
            .docker_labels()
            .is_some_and(|labels| !labels.is_empty())
        || !container.ulimits().is_empty()
        || logs.log_driver() != &LogDriver::Awslogs
        || logs.options() != Some(&expected_log_options)
        || !logs.secret_options().is_empty()
        || container.health_check().is_some()
        || !container.system_controls().is_empty()
        || !container.resource_requirements().is_empty()
        || container.firelens_configuration().is_some()
        || !container.credential_specs().is_empty()
    {
        bail!("stored container differs from the expected runner contract");
    }
    Ok(())
}

fn scope_tag(key: &str, value: &str) -> Tag {
    Tag::builder().key(key).value(value).build()
}

fn secret_tag(key: &str, value: &str) -> SecretTag {
    SecretTag::builder().key(key).value(value).build()
}

fn image_digest(image: &str) -> anyhow::Result<&str> {
    let (_, digest) = image
        .rsplit_once("@sha256:")
        .context("runner image must be pinned by sha256 digest")?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("runner image has an invalid sha256 digest");
    }
    Ok(digest)
}

fn task_family(attempt_id: &str) -> anyhow::Result<String> {
    if attempt_id.is_empty()
        || !attempt_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("attempt id cannot form an ECS task family");
    }
    Ok(format!("{TASK_FAMILY_PREFIX}{attempt_id}"))
}

fn task_definition_family(arn: &str) -> Option<&str> {
    arn.rsplit_once('/')?
        .1
        .rsplit_once(':')
        .map(|(family, _)| family)
}

fn secret_name(
    cluster_arn: &str,
    attempt_id: &str,
    secret_name_key: &[u8; 32],
) -> anyhow::Result<String> {
    let cluster = cluster_arn
        .rsplit_once('/')
        .map(|(_, cluster)| cluster)
        .filter(|cluster| !cluster.is_empty())
        .context("ECS cluster ARN is missing its cluster name")?;
    task_family(attempt_id)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret_name_key)
        .expect("a fixed-size HMAC key is always valid");
    mac.update(attempt_id.as_bytes());
    let suffix = hex::encode(mac.finalize().into_bytes());
    Ok(format!(
        "scope-vcs/{cluster}/attempts/{attempt_id}-{}",
        &suffix[..32]
    ))
}

#[cfg(test)]
mod tests;
