use super::*;

const IMAGE: &str =
    "ghcr.io/scope/checks@sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const SECRET_ARN: &str = "arn:aws:secretsmanager:us-east-1:123456789012:secret:scope/attempt";

fn settings() -> CloudExecutionSettings {
    CloudExecutionSettings {
        api_url: "https://scope.test".to_string(),
        aws_region: "us-east-1".to_string(),
        ecs_cluster_arn: "arn:aws:ecs:us-east-1:123456789012:cluster/scope".to_string(),
        ecs_subnet_ids: vec!["subnet-a".to_string()],
        ecs_security_group_id: "sg-a".to_string(),
        ecs_execution_role_arn: "arn:aws:iam::123456789012:role/scope-task-execution".to_string(),
        ecs_log_group: "/scope/production/runner".to_string(),
        runtime_version: "test".to_string(),
        max_concurrency: 1,
    }
}

fn task_definition(image: &str) -> TaskDefinition {
    let settings = settings();
    TaskDefinition::builder()
        .network_mode(NetworkMode::Awsvpc)
        .requires_compatibilities(Compatibility::Fargate)
        .cpu(TASK_CPU)
        .memory(TASK_MEMORY)
        .execution_role_arn(&settings.ecs_execution_role_arn)
        .runtime_platform(runtime_platform())
        .container_definitions(
            runner_container(
                image,
                &settings.aws_region,
                &settings.ecs_log_group,
                SECRET_ARN,
            )
            .unwrap(),
        )
        .build()
}

#[test]
fn task_family_is_unique_to_the_attempt() {
    assert_eq!(
        task_family("attempt_123").unwrap(),
        "scope-runner-attempt_123"
    );
}

#[test]
fn task_family_rejects_unsafe_attempt_ids() {
    assert!(task_family("").is_err());
    assert!(task_family("attempt/unsafe").is_err());
}

#[test]
fn images_must_remain_digest_pinned() {
    assert!(image_digest("ghcr.io/scope/checks:latest").is_err());
    assert!(image_digest("ghcr.io/scope/checks@sha256:abcd").is_err());
}

#[test]
fn task_definition_arn_must_match_the_exact_family() {
    assert_eq!(
        task_definition_family(
            "arn:aws:ecs:us-east-1:123456789012:task-definition/scope-runner-attempt_abcd:7"
        ),
        Some("scope-runner-attempt_abcd")
    );
    assert_eq!(task_definition_family("not-an-arn"), None);
}

#[test]
fn task_definition_preserves_image_entrypoint_and_log_destination() {
    let container =
        runner_container(IMAGE, "us-east-1", "/scope/production/runner", SECRET_ARN).unwrap();
    assert_eq!(container.name(), Some(CONTAINER_NAME));
    assert_eq!(container.image(), Some(IMAGE));
    assert_eq!(container.entry_point(), [RUNTIME_ENTRYPOINT]);
    assert_eq!(container.secrets()[0].name(), BOOTSTRAP_SECRET_ENV);
    assert_eq!(container.secrets()[0].value_from(), SECRET_ARN);
    let logs = container.log_configuration().unwrap();
    assert_eq!(logs.log_driver(), &LogDriver::Awslogs);
    assert_eq!(
        logs.options()
            .unwrap()
            .get("awslogs-group")
            .map(String::as_str),
        Some("/scope/production/runner")
    );
    assert_eq!(
        logs.options()
            .unwrap()
            .get("awslogs-region")
            .map(String::as_str),
        Some("us-east-1")
    );
}

#[test]
fn task_override_contains_only_non_secret_attempt_runtime_values() {
    let overrides = task_override("https://scope.test", "attempt_1", 86_400);
    let container = &overrides.container_overrides()[0];
    assert_eq!(container.name(), Some(CONTAINER_NAME));
    let environment = container
        .environment()
        .iter()
        .map(|value| (value.name().unwrap(), value.value().unwrap()))
        .collect::<HashMap<_, _>>();
    assert_eq!(environment.len(), 3);
    assert_eq!(environment["SCOPE_API_URL"], "https://scope.test");
    assert_eq!(environment["SCOPE_ATTEMPT_ID"], "attempt_1");
    assert_eq!(environment["SCOPE_ATTEMPT_DEADLINE_UNIX"], "86400");
    assert!(!environment.contains_key(BOOTSTRAP_SECRET_ENV));
}

#[test]
fn stored_task_definition_must_match_the_whole_launch_contract() {
    let settings = settings();
    assert!(
        verify_task_definition_contract(&task_definition(IMAGE), IMAGE, SECRET_ARN, &settings)
            .is_ok()
    );

    let mut altered = task_definition(IMAGE);
    altered.task_role_arn = Some("arn:aws:iam::123456789012:role/unexpected".to_string());
    assert!(verify_task_definition_contract(&altered, IMAGE, SECRET_ARN, &settings).is_err());

    let mut normalized = task_definition(IMAGE);
    let normalized_container = &mut normalized.container_definitions.as_mut().unwrap()[0];
    normalized_container.cpu = 0;
    normalized_container.port_mappings = Some(Vec::new());
    normalized_container.environment = Some(Vec::new());
    assert!(verify_task_definition_contract(&normalized, IMAGE, SECRET_ARN, &settings).is_ok());

    let mut altered = task_definition(IMAGE);
    altered.container_definitions.as_mut().unwrap()[0].command =
        Some(vec!["unexpected".to_string()]);
    assert!(verify_task_definition_contract(&altered, IMAGE, SECRET_ARN, &settings).is_err());

    let mut altered = task_definition(IMAGE);
    altered.container_definitions.as_mut().unwrap()[0].secrets = Some(Vec::new());
    assert!(verify_task_definition_contract(&altered, IMAGE, SECRET_ARN, &settings).is_err());
}

#[test]
fn retry_is_unblocked_only_after_ecs_reports_the_task_stopped() {
    let running = Task::builder().last_status("RUNNING").build();
    assert!(!task_has_stopped(&[running], &[], "task-1").unwrap());

    let stopped = Task::builder().last_status("STOPPED").build();
    assert!(task_has_stopped(&[stopped], &[], "task-1").unwrap());

    let missing = Failure::builder().arn("task-1").reason("MISSING").build();
    assert!(task_has_stopped(&[], &[missing], "task-1").unwrap());

    let denied = Failure::builder()
        .arn("task-1")
        .reason("ACCESS_DENIED")
        .build();
    assert!(task_has_stopped(&[], &[denied], "task-1").is_err());
}

#[test]
fn ambiguous_start_polling_uses_bounded_exponential_backoff() {
    let mut delay = CONSISTENCY_INITIAL_DELAY;
    let mut observed = Vec::new();
    for _ in 0..6 {
        observed.push(delay.as_secs());
        delay = next_consistency_delay(delay);
    }
    assert_eq!(observed, [2, 4, 8, 16, 30, 30]);
}
