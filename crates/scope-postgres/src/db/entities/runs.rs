use super::*;

pub mod runner {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_runners")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub owner_user_id: String,
        #[sea_orm(unique)]
        pub secret_hash: String,
        pub version: String,
        pub protocol_version: i32,
        pub capabilities: Json,
        pub max_concurrent_jobs: i32,
        pub enabled: bool,
        pub created_at_unix: i64,
        pub last_seen_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(runner: &Runner) -> Result<Self, PostgresError> {
            Ok(Self {
                id: runner.id.clone(),
                owner_user_id: runner.owner_user_id.clone(),
                secret_hash: runner.secret_hash.clone(),
                version: runner.version.clone(),
                protocol_version: u32_to_i32(runner.protocol_version, "runner protocol version")?,
                capabilities: encode_json(&runner.capabilities)?,
                max_concurrent_jobs: i32::from(runner.max_concurrent_jobs.get()),
                enabled: runner.enabled,
                created_at_unix: u64_to_i64(runner.created_at_unix, "runner creation time")?,
                last_seen_at_unix: runner
                    .last_seen_at_unix
                    .map(|value| u64_to_i64(value, "runner last-seen time"))
                    .transpose()?,
            })
        }

        pub fn try_into_domain(self) -> Result<Runner, PostgresError> {
            Runner::restore(
                self.id,
                self.owner_user_id,
                self.secret_hash,
                self.version,
                i32_to_u32(self.protocol_version, "runner protocol version")?,
                decode_json::<RunnerCapabilities>(self.capabilities)?,
                scope_domain::runs::runner::RunnerMaxConcurrentJobs::new(
                    u8::try_from(self.max_concurrent_jobs).map_err(|_| {
                        PostgresError::invalid_input("persisted runner capacity is invalid")
                    })?,
                )
                .map_err(PostgresError::invalid_input)?,
                self.enabled,
                i64_to_u64(self.created_at_unix, "runner creation time")?,
                self.last_seen_at_unix
                    .map(|value| i64_to_u64(value, "runner last-seen time"))
                    .transpose()?,
            )
            .map_err(PostgresError::invalid_input)
        }
    }
}

pub mod runner_grant {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_runner_grants")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub runner_id: String,
        pub name: String,
        pub granted_by_user_id: String,
        pub created_at_unix: i64,
        pub revoked_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(grant: &RunnerGrant) -> Result<Self, PostgresError> {
            Ok(Self {
                repo_id: grant.repository_id.clone(),
                runner_id: grant.runner_id.clone(),
                name: grant.name.as_str().to_string(),
                granted_by_user_id: grant.granted_by_user_id.clone(),
                created_at_unix: u64_to_i64(grant.created_at_unix, "runner grant creation time")?,
                revoked_at_unix: grant
                    .revoked_at_unix
                    .map(|value| u64_to_i64(value, "runner grant revocation time"))
                    .transpose()?,
            })
        }

        pub fn try_into_domain(self) -> Result<RunnerGrant, PostgresError> {
            RunnerGrant::restore(
                self.repo_id,
                self.runner_id,
                RunnerName::parse(self.name).map_err(PostgresError::invalid_input)?,
                self.granted_by_user_id,
                i64_to_u64(self.created_at_unix, "runner grant creation time")?,
                self.revoked_at_unix
                    .map(|value| i64_to_u64(value, "runner grant revocation time"))
                    .transpose()?,
            )
            .map_err(PostgresError::invalid_input)
        }
    }
}

pub mod workflow_revision {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_workflow_revisions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub digest: String,
        pub definition: Json,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(
            revision: &WorkflowRevision,
            created_at_unix: u64,
        ) -> Result<Self, PostgresError> {
            Ok(Self {
                digest: revision.digest().to_string(),
                definition: encode_json(revision.definition())?,
                created_at_unix: u64_to_i64(created_at_unix, "workflow revision creation time")?,
            })
        }

        pub fn try_into_domain(
            self,
            identity: WorkflowIdentity,
        ) -> Result<WorkflowRevision, PostgresError> {
            let persisted_digest = self.digest;
            let definition = decode_json::<CompiledWorkflow>(self.definition)?;
            let revision = WorkflowRevision::new(identity, definition)
                .map_err(PostgresError::invalid_input)?;
            if revision.digest() != persisted_digest {
                return Err(PostgresError::internal_message(
                    "persisted workflow revision digest does not match its definition",
                ));
            }
            Ok(revision)
        }
    }
}

pub mod push_trigger_evaluation {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_push_trigger_evaluations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub change_version: i64,
        pub head_oid: String,
        pub state: String,
        pub message: Option<String>,
        pub checks: Json,
        pub created_at_unix: i64,
        pub completed_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(evaluation: &PushTriggerEvaluation) -> Result<Self, PostgresError> {
            Ok(Self {
                repo_id: evaluation.repository_id.clone(),
                change_version: u64_to_i64(
                    evaluation.change_version,
                    "push trigger change version",
                )?,
                head_oid: evaluation.head_oid.clone(),
                state: encode_enum(evaluation.state)?,
                message: evaluation.message.clone(),
                checks: encode_json(&evaluation.checks)?,
                created_at_unix: u64_to_i64(
                    evaluation.created_at_unix,
                    "push trigger creation time",
                )?,
                completed_at_unix: evaluation
                    .completed_at_unix
                    .map(|value| u64_to_i64(value, "push trigger completion time"))
                    .transpose()?,
            })
        }

        pub fn try_into_domain(self) -> Result<PushTriggerEvaluation, PostgresError> {
            Ok(PushTriggerEvaluation {
                repository_id: self.repo_id,
                change_version: i64_to_u64(self.change_version, "push trigger change version")?,
                head_oid: self.head_oid,
                state: decode_enum(self.state)?,
                message: self.message,
                checks: decode_json(self.checks)?,
                created_at_unix: i64_to_u64(self.created_at_unix, "push trigger creation time")?,
                completed_at_unix: self
                    .completed_at_unix
                    .map(|value| i64_to_u64(value, "push trigger completion time"))
                    .transpose()?,
            })
        }
    }
}

pub mod run {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_runs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub idempotency_key: String,
        pub repo_id: String,
        pub workflow_path: String,
        pub workflow_revision_digest: String,
        pub trigger: String,
        pub requested_by_user_id: Option<String>,
        pub source: Json,
        pub runner_override_name: Option<String>,
        pub state: String,
        pub cancellation_requested: bool,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
        pub completed_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(run: &Run) -> Result<Self, PostgresError> {
            Ok(Self {
                id: run.id.clone(),
                idempotency_key: run.idempotency_key.clone(),
                repo_id: run.workflow.repository_id().to_string(),
                workflow_path: run.workflow.path().as_str().to_string(),
                workflow_revision_digest: run.workflow_revision_digest.clone(),
                trigger: encode_enum(run.trigger)?,
                requested_by_user_id: run.requested_by_user_id.clone(),
                source: encode_json(&run.source)?,
                runner_override_name: run.runner_override.as_ref().and_then(
                    |runner| match runner {
                        RunnerSelector::Any => None,
                        RunnerSelector::Named(name) => Some(name.clone()),
                    },
                ),
                state: encode_enum(run.state)?,
                cancellation_requested: run.cancellation_requested,
                created_at_unix: u64_to_i64(run.created_at_unix, "run creation time")?,
                updated_at_unix: u64_to_i64(run.updated_at_unix, "run update time")?,
                completed_at_unix: run
                    .completed_at_unix
                    .map(|value| u64_to_i64(value, "run completion time"))
                    .transpose()?,
            })
        }

        pub fn try_into_domain(self) -> Result<Run, PostgresError> {
            let workflow = WorkflowIdentity::new(
                self.repo_id,
                WorkflowPath::parse(self.workflow_path).map_err(PostgresError::invalid_input)?,
            )
            .map_err(PostgresError::invalid_input)?;
            let runner_override = self
                .runner_override_name
                .map(RunnerSelector::named)
                .transpose()
                .map_err(PostgresError::invalid_input)?;
            Run::restore(
                self.id,
                self.idempotency_key,
                workflow,
                self.workflow_revision_digest,
                decode_enum::<RunTrigger>(self.trigger)?,
                self.requested_by_user_id,
                decode_json::<RunSource>(self.source)?,
                runner_override,
                decode_enum::<RunState>(self.state)?,
                self.cancellation_requested,
                i64_to_u64(self.created_at_unix, "run creation time")?,
                i64_to_u64(self.updated_at_unix, "run update time")?,
                self.completed_at_unix
                    .map(|value| i64_to_u64(value, "run completion time"))
                    .transpose()?,
            )
            .map_err(PostgresError::invalid_input)
        }
    }
}

pub mod run_job {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_run_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub run_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub job_key: String,
        pub desired_runner_name: Option<String>,
        pub pinned_container_image: Option<String>,
        pub state: String,
        pub last_attempt_number: i32,
        pub current_attempt_id: Option<String>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
        pub completed_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(job: &RunJob) -> Result<Self, PostgresError> {
            Ok(Self {
                run_id: job.run_id.clone(),
                job_key: job.key.as_str().to_string(),
                desired_runner_name: match &job.desired_runner {
                    RunnerSelector::Any => None,
                    RunnerSelector::Named(name) => Some(name.clone()),
                },
                pinned_container_image: job
                    .pinned_container_image
                    .as_ref()
                    .map(|image| image.as_str().to_string()),
                state: encode_enum(job.state)?,
                last_attempt_number: u32_to_i32(
                    job.last_attempt_number,
                    "run job last attempt number",
                )?,
                current_attempt_id: job.current_attempt_id.clone(),
                created_at_unix: u64_to_i64(job.created_at_unix, "run job creation time")?,
                updated_at_unix: u64_to_i64(job.updated_at_unix, "run job update time")?,
                completed_at_unix: job
                    .completed_at_unix
                    .map(|value| u64_to_i64(value, "run job completion time"))
                    .transpose()?,
            })
        }

        pub fn try_into_domain(self) -> Result<RunJob, PostgresError> {
            let desired_runner = match self.desired_runner_name {
                Some(name) => RunnerSelector::named(name).map_err(PostgresError::invalid_input)?,
                None => RunnerSelector::Any,
            };
            RunJob::restore(
                self.run_id,
                WorkflowJobId::parse(self.job_key).map_err(PostgresError::invalid_input)?,
                desired_runner,
                self.pinned_container_image
                    .map(PinnedContainerImage::parse)
                    .transpose()
                    .map_err(PostgresError::invalid_input)?,
                decode_enum::<RunJobState>(self.state)?,
                i32_to_u32(self.last_attempt_number, "run job last attempt number")?,
                self.current_attempt_id,
                i64_to_u64(self.created_at_unix, "run job creation time")?,
                i64_to_u64(self.updated_at_unix, "run job update time")?,
                self.completed_at_unix
                    .map(|value| i64_to_u64(value, "run job completion time"))
                    .transpose()?,
            )
            .map_err(PostgresError::invalid_input)
        }
    }
}

pub mod run_attempt {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_run_attempts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub run_id: String,
        pub job_key: String,
        pub number: i32,
        pub runner_id: String,
        pub runner_name: String,
        #[sea_orm(unique)]
        pub token_hash: String,
        pub token_expires_at_unix: i64,
        pub state: String,
        pub lease_expires_at_unix: i64,
        pub last_heartbeat_at_unix: i64,
        pub created_at_unix: i64,
        pub started_at_unix: Option<i64>,
        pub completed_at_unix: Option<i64>,
        pub terminal_reason: Option<Json>,
        pub log_bytes: i64,
        pub first_truncated_step_index: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(attempt: &RunAttempt) -> Result<Self, PostgresError> {
            Ok(Self {
                id: attempt.id.clone(),
                run_id: attempt.run_id.clone(),
                job_key: attempt.job_key.as_str().to_string(),
                number: u32_to_i32(attempt.number, "run attempt number")?,
                runner_id: attempt.runner_id.clone(),
                runner_name: attempt.runner_name.clone(),
                token_hash: attempt.token_hash.clone(),
                token_expires_at_unix: u64_to_i64(
                    attempt.token_expires_at_unix,
                    "attempt token expiry time",
                )?,
                state: encode_enum(attempt.state)?,
                lease_expires_at_unix: u64_to_i64(
                    attempt.lease_expires_at_unix,
                    "attempt lease expiry time",
                )?,
                last_heartbeat_at_unix: u64_to_i64(
                    attempt.last_heartbeat_at_unix,
                    "attempt heartbeat time",
                )?,
                created_at_unix: u64_to_i64(attempt.created_at_unix, "attempt creation time")?,
                started_at_unix: attempt
                    .started_at_unix
                    .map(|value| u64_to_i64(value, "attempt start time"))
                    .transpose()?,
                completed_at_unix: attempt
                    .completed_at_unix
                    .map(|value| u64_to_i64(value, "attempt completion time"))
                    .transpose()?,
                terminal_reason: attempt
                    .terminal_reason
                    .as_ref()
                    .map(encode_json)
                    .transpose()?,
                log_bytes: u64_to_i64(attempt.log_bytes, "attempt log byte count")?,
                first_truncated_step_index: attempt
                    .first_truncated_step_index
                    .map(|value| u32_to_i32(value, "first truncated step index"))
                    .transpose()?,
            })
        }

        pub fn try_into_domain(self) -> Result<RunAttempt, PostgresError> {
            RunAttempt::restore(
                self.id,
                self.run_id,
                WorkflowJobId::parse(self.job_key).map_err(PostgresError::invalid_input)?,
                i32_to_u32(self.number, "run attempt number")?,
                self.runner_id,
                self.runner_name,
                self.token_hash,
                i64_to_u64(self.token_expires_at_unix, "attempt token expiry time")?,
                decode_enum::<AttemptState>(self.state)?,
                i64_to_u64(self.lease_expires_at_unix, "attempt lease expiry time")?,
                i64_to_u64(self.last_heartbeat_at_unix, "attempt heartbeat time")?,
                i64_to_u64(self.created_at_unix, "attempt creation time")?,
                self.started_at_unix
                    .map(|value| i64_to_u64(value, "attempt start time"))
                    .transpose()?,
                self.completed_at_unix
                    .map(|value| i64_to_u64(value, "attempt completion time"))
                    .transpose()?,
                self.terminal_reason
                    .map(decode_json::<AttemptTerminalReason>)
                    .transpose()?,
                i64_to_u64(self.log_bytes, "attempt log byte count")?,
                self.first_truncated_step_index
                    .map(|value| i32_to_u32(value, "first truncated step index"))
                    .transpose()?,
            )
            .map_err(PostgresError::invalid_input)
        }
    }
}

pub mod run_attempt_step {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_run_attempt_steps")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub attempt_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub step_index: i32,
        pub state: String,
        pub started_at_unix: Option<i64>,
        pub completed_at_unix: Option<i64>,
        pub exit_code: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(step: &RunAttemptStep) -> Result<Self, PostgresError> {
            Ok(Self {
                attempt_id: step.attempt_id.clone(),
                step_index: u32_to_i32(step.step_index, "run attempt step index")?,
                state: encode_enum(step.state)?,
                started_at_unix: step
                    .started_at_unix
                    .map(|value| u64_to_i64(value, "run attempt step start time"))
                    .transpose()?,
                completed_at_unix: step
                    .completed_at_unix
                    .map(|value| u64_to_i64(value, "run attempt step completion time"))
                    .transpose()?,
                exit_code: step.exit_code,
            })
        }

        pub fn try_into_domain(self) -> Result<RunAttemptStep, PostgresError> {
            RunAttemptStep::restore(
                self.attempt_id,
                i32_to_u32(self.step_index, "run attempt step index")?,
                decode_enum::<StepState>(self.state)?,
                self.started_at_unix
                    .map(|value| i64_to_u64(value, "run attempt step start time"))
                    .transpose()?,
                self.completed_at_unix
                    .map(|value| i64_to_u64(value, "run attempt step completion time"))
                    .transpose()?,
                self.exit_code,
            )
            .map_err(PostgresError::invalid_input)
        }
    }
}

pub mod run_log {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_run_logs")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub position: i64,
        pub run_id: String,
        pub attempt_id: String,
        pub step_index: i32,
        pub sequence: i64,
        pub text: String,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(run_id: &str, chunk: &RunLogChunk) -> Result<Self, PostgresError> {
            Ok(Self {
                position: 0,
                run_id: run_id.to_string(),
                attempt_id: chunk.attempt_id.clone(),
                step_index: u32_to_i32(chunk.step_index, "run log step index")?,
                sequence: u64_to_i64(chunk.sequence, "run log sequence")?,
                text: chunk.text.clone(),
                created_at_unix: u64_to_i64(chunk.created_at_unix, "run log creation time")?,
            })
        }

        pub fn try_into_domain(self) -> Result<RunLogChunk, PostgresError> {
            RunLogChunk::new(
                self.attempt_id,
                i32_to_u32(self.step_index, "run log step index")?,
                i64_to_u64(self.sequence, "run log sequence")?,
                self.text,
                i64_to_u64(self.created_at_unix, "run log creation time")?,
            )
            .map_err(PostgresError::invalid_input)
        }
    }
}
