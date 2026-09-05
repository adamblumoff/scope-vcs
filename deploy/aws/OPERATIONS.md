# Cloud runner operations

CloudFormation owns the runner VPC, networking, security group, ECS cluster, log group, task execution role, private ECR checks-image repository, dispatcher IAM user, exact private-registry secret grant, and optional budget. Production also creates GitHub OIDC administration roles. Do not create parallel resources in the AWS Console.

Fargate On-Demand is the default. Each Fargate task gets a public IPv4 address because the runner must reach ECR, the Scope API, the cache, and source hosts. The security group has no inbound rules and permits outbound HTTPS only. There is no NAT gateway or idle compute cost. Checks images live in private ECR in the same region as Fargate and are published as SOCI v2 image indexes so Fargate can lazy-load their filesystems.

The opt-in Managed Instances mode creates private subnets, one NAT gateway, and an ECS Managed Instances capacity provider. The provider launches the configured four-vCPU instance type on demand and starts idle scale-in immediately. Managed tasks do not receive public IP addresses. Set a distinct task-family prefix for every environment so a staging dispatcher cannot deregister production task definitions.

The worker registers one `scope-runner-<attempt ID>` task definition per attempt because ECS cannot override either the container image or secret references in `RunTask`. The definition contains the digest-pinned image and a reference to a per-attempt Secrets Manager bootstrap credential. When `SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN` is set, it also contains that exact ARN as ECS repository credentials. Neither credential value is placed in the ECS task override or returned by `DescribeTasks`. After ECS reports the task stopped, the worker deregisters the task definition and force-deletes the one-use bootstrap secret.

Each task is also keyed by its Scope attempt ID. The runtime has an absolute 24-hour watchdog, independent of the worker and heartbeat lease. The worker also hard-expires the database attempt and waits for ECS to report the old task as `STOPPED` before the job can dispatch another attempt. If `RunTask` succeeds but its response is lost, the worker polls for the task by attempt ID for five minutes, using bounded exponential backoff for ECS eventual consistency, before concluding no task was created. Cleanup claims run concurrently and are held for fifteen minutes so another worker cannot race that reconciliation.

## Prerequisites

- AWS CLI v2 authenticated to the production account
- GitHub CLI authenticated with repository administration access
- permission to manage CloudFormation, VPC, ECS, IAM, CloudWatch Logs, and AWS Budgets
- Railway CLI authenticated to the Scope production project when setting worker variables

Check the AWS identity before making a plan:

```bash
aws sts get-caller-identity
```

Never deploy this stack from the AWS root user. Use an administrative role with MFA for the initial stack and a CI deployment role for later changes.

## Managed Instances staging experiment

Use an isolated stack and leave staging cloud execution disabled until the stack, credentials, and candidate worker are healthy. The experiment uses the production checks image only as a read-only input.

```bash
STACK_NAME=scope-cloud-runner-staging-mi-experiment \
ENVIRONMENT_NAME=staging \
RUNNER_CAPACITY=MANAGED_INSTANCES \
MANAGED_INSTANCE_TYPE=m8a.xlarge \
TASK_MEMORY_MIB=14336 \
TASK_FAMILY_PREFIX=scope-staging-runner- \
ADDITIONAL_CHECKS_IMAGE_REPOSITORY_ARN=arn:aws:ecr:us-east-1:<account-id>:repository/scope-vcs/production/checks \
ENABLE_GITHUB_ADMINISTRATION=false \
EXISTING_GITHUB_OIDC_PROVIDER_ARN=arn:aws:iam::<account-id>:oidc-provider/token.actions.githubusercontent.com \
deploy/aws/apply-cloud-runner.sh plan
```

Review the complete change set, then repeat the same environment variables and apply its exact ARN. Create a new dispatcher access key for this stack. Do not reuse the production dispatcher credentials or secret-name key.

Configure only the staging worker with the stack outputs. Keep `SCOPE_CLOUD_RUNS_MAX_CONCURRENCY=20` and set `SCOPE_CLOUD_RUNS_ENABLED=1` last. The worker also needs:

```text
SCOPE_ECS_CAPACITY_PROVIDER   <- RunnerCapacityProvider
SCOPE_ECS_TASK_MEMORY_MIB     <- RunnerTaskMemoryMiB
SCOPE_ECS_TASK_FAMILY_PREFIX  <- RunnerTaskFamilyPrefix
```

After collecting the experiment, set `SCOPE_CLOUD_RUNS_ENABLED=0`, stop remaining experimental tasks, remove the staging dispatcher access keys, and delete the isolated stack. The checks-image repository has a retain policy, so inspect and delete the empty staging repository separately.

The GitHub OIDC provider is account-global. Check for an existing provider before the first stack update:

```bash
aws iam list-open-id-connect-providers \
  --query "OpenIDConnectProviderList[?contains(Arn, 'token.actions.githubusercontent.com')].Arn" \
  --output text
```

If this prints an ARN, pass it as `EXISTING_GITHUB_OIDC_PROVIDER_ARN` when planning and applying. Otherwise the stack creates and owns the provider.

## Validate and preview

The script defaults to `us-east-1`, stack `scope-cloud-runner-production`, project `scope-vcs`, and environment `production`.

```bash
deploy/aws/apply-cloud-runner.sh validate
deploy/aws/apply-cloud-runner.sh plan
```

For an account that already has the GitHub provider:

```bash
EXISTING_GITHUB_OIDC_PROVIDER_ARN=arn:aws:iam::<account ID>:oidc-provider/token.actions.githubusercontent.com \
deploy/aws/apply-cloud-runner.sh plan
```

`plan` creates a CloudFormation change set, waits for AWS to prepare it, and prints every resource action. It does not execute the change set. For the first deployment, discard an unused preview by deleting its empty `REVIEW_IN_PROGRESS` stack:

```bash
aws cloudformation delete-stack \
  --region us-east-1 \
  --stack-name scope-cloud-runner-production
```

For later updates, delete only the unused change set with `aws cloudformation delete-change-set --change-set-name <change-set ARN>`.

Set `BUDGET_NOTIFICATION_EMAIL` to create a monthly project-tag budget. `MONTHLY_BUDGET_USD` defaults to 100.

```bash
BUDGET_NOTIFICATION_EMAIL=ops@example.com \
MONTHLY_BUDGET_USD=100 \
deploy/aws/apply-cloud-runner.sh plan
```

AWS Budgets can filter on the `Project=scope-vcs` tag only after the account activates that cost allocation tag. Check and activate it through the CLI:

```bash
aws ce list-cost-allocation-tags \
  --status Active \
  --tag-keys Project

aws ce update-cost-allocation-tags-status \
  --cost-allocation-tags-status TagKey=Project,Status=Active
```

Cost allocation tags can take up to 24 hours to appear. Omit `BUDGET_NOTIFICATION_EMAIL` until the tag is available if this is a new AWS account.

## Apply

`apply` validates the template, creates and prints a fresh change set, executes it, waits for the stack, and prints its outputs.

If an initial create reaches `ROLLBACK_COMPLETE`, the next `plan` or `apply` prints the latest stack events, deletes only that rolled-back empty stack, waits for deletion, and creates a fresh change set. Later update rollbacks remain intact for inspection and rollback.

```bash
BUDGET_NOTIFICATION_EMAIL=ops@example.com \
deploy/aws/apply-cloud-runner.sh apply
```

To execute the exact change set returned by `plan`, pass its ARN. This is required for the first deployment if the stack remains in `REVIEW_IN_PROGRESS` after a preview:

```bash
deploy/aws/apply-cloud-runner.sh apply <change-set ARN>
```

The script exits successfully when the stack already matches the template.

Apply this infrastructure before merging a workflow that publishes to ECR. Then configure the one non-secret GitHub Actions variable from the CloudFormation output:

```bash
publisher_role_arn="$(aws cloudformation describe-stacks \
  --region us-east-1 \
  --stack-name scope-cloud-runner-production \
  --query "Stacks[0].Outputs[?OutputKey=='ChecksImagePublisherRoleArn'].OutputValue | [0]" \
  --output text)"

gh variable set SCOPE_CHECKS_IMAGE_AWS_ROLE_ARN \
  --repo scope-vcs/scope-vcs \
  --body "$publisher_role_arn"
```

GitHub receives temporary AWS credentials through OIDC. There is no AWS access key to create or store for image publishing. The role accepts only this repository's branch refs and the `scope-checks-image.yml` reusable workflow, and it can write only the checks-image repository.

Configure the infrastructure role at the same time:

```bash
infrastructure_role_arn="$(aws cloudformation describe-stacks \
  --region us-east-1 \
  --stack-name scope-cloud-runner-production \
  --query "Stacks[0].Outputs[?OutputKey=='GitHubInfrastructureRoleArn'].OutputValue | [0]" \
  --output text)"

gh variable set SCOPE_AWS_INFRASTRUCTURE_ROLE_ARN \
  --repo scope-vcs/scope-vcs \
  --body "$infrastructure_role_arn"
```

After this one-time bootstrap, use the `Scope AWS Infrastructure` GitHub workflow for persistent, keyless administration. Run `plan`, review its change-set ARN, then run `apply` with that exact ARN. The infrastructure role has administrator permissions because the stack owns IAM, networking, compute, storage, logs, and budgets; its trust policy restricts assumption to the immutable repository ID and this workflow on `main`.

## Configure private registry credentials

Public images need no registry credential. Leave `SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN` unset in both GitHub and Railway to keep `repositoryCredentials` out of the ECS task definition.

Private registries use one Secrets Manager secret with this JSON shape:

```json
{"username":"registry-user","password":"registry-password-or-token"}
```

Create the secret in the runner's AWS region through a secure shell and encrypt it with the AWS-managed `aws/secretsmanager` key. A customer-managed key also requires an exact `kms:Decrypt` grant and is not supported by this stack. Do not put the username, password, token, or JSON document in the repository, a ticket, shell history, or a GitHub variable. Only the secret ARN is configuration.

Set the ARN as the repository variable used by the infrastructure workflow:

```bash
gh variable set SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN \
  --repo scope-vcs/scope-vcs \
  --body "$registry_credentials_secret_arn"
```

Run the `Scope AWS Infrastructure` workflow with `plan`, review the exact change set, then apply it. The task execution role must receive its exact-secret grant before any worker starts emitting task definitions that reference the secret.

After the stack update succeeds, set the same ARN on the Railway worker and deploy the worker:

```bash
railway variable set \
  --project "$railway_project_id" \
  --environment "$railway_environment_id" \
  --service "$railway_worker_service_id" \
  "SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN=$registry_credentials_secret_arn"
```

This is one global worker setting. When set, every task definition receives the credential ARN. It does not select credentials by image host and cannot mix credential-free public pulls with authenticated pulls from one worker. Leave it unset for public-only execution. Supporting mixed registry modes requires an explicit workflow contract and is outside this cutover.

To rotate credentials, write a new secret version under the same ARN, run one private-image canary, then revoke the old registry token. The worker does not need a configuration change when the ARN stays the same.

## Create the dispatcher credentials

CloudFormation creates the least-privilege IAM user but does not create an access key. This keeps the secret out of stack outputs and CloudFormation event history. Create one key through the CLI after the stack succeeds:

```bash
dispatcher_user="$(aws cloudformation describe-stacks \
  --region us-east-1 \
  --stack-name scope-cloud-runner-production \
  --query "Stacks[0].Outputs[?OutputKey=='RailwayDispatcherUserName'].OutputValue | [0]" \
  --output text)"

aws iam create-access-key --user-name "$dispatcher_user"
```

Copy the returned access key ID and secret directly into Railway. Do not save the JSON to the repository or shell history. Generate a separate secret-name key in the secure shell that will update Railway:

```bash
ecs_secret_name_key="$(openssl rand -hex 32)"
```

The worker also needs the six non-secret stack outputs:

```text
AWS_REGION                    <- AwsRegion
SCOPE_ECS_CLUSTER_ARN         <- RunnerClusterArn
SCOPE_ECS_SUBNET_IDS          <- RunnerSubnetIds
SCOPE_ECS_SECURITY_GROUP_ID   <- RunnerSecurityGroupId
SCOPE_ECS_EXECUTION_ROLE_ARN  <- RunnerExecutionRoleArn
SCOPE_ECS_LOG_GROUP           <- RunnerLogGroupName
```

Set them with `railway variable set` in the production worker service. Set `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and `SCOPE_ECS_SECRET_NAME_KEY="$ecs_secret_name_key"` in the same command or through a secure, non-recorded shell session. Do not paste secrets into command examples, tickets, or logs.

The dispatcher's policy permits only these operations:

- register, list, deregister, and tag `scope-runner-attempt_*` task definitions, without permission to inspect their secret references
- run those task definitions only in this cluster
- find an ambiguously started task by its Scope attempt ID
- describe, stop, and tag tasks in this cluster
- create and delete project-tagged per-attempt bootstrap secrets without permission to read them
- pass the exact task execution role to ECS

The policy explicitly denies direct `GetSecretValue`, batch get, describe, and list operations. Those denies block the dispatcher from reading the registry credential or any stored bootstrap credential through the Secrets Manager API.

The task execution role can read only secrets under this cluster's per-attempt prefix and the configured registry credential ARN. The ECS agent uses those permissions to inject the bootstrap value and authenticate the image pull. Secret names contain an HMAC suffix derived from `SCOPE_ECS_SECRET_NAME_KEY`; the AWS dispatcher identity can neither list secrets nor inspect registered task definitions, so possession of that access key alone cannot discover another attempt's bootstrap reference. In task definitions generated by this worker, the registry secret is used only as `repositoryCredentials` and is not injected into the container. The tasks receive no task IAM role, and runner code has no AWS credentials.

The direct API deny is not a security boundary against malicious code that holds the dispatcher credentials. That identity can register and run task definitions with the execution role, so it could ask ECS to inject a known secret ARN into a container. Preventing that requires a trusted task-definition registration service or pre-registered definitions. Treat the Railway worker and its dispatcher credentials as trusted production control-plane access.

## Observe a real run

Tail container output:

```bash
aws logs tail /scope-vcs/production/cloud-runner \
  --region us-east-1 \
  --follow
```

Inspect active and stopped tasks:

```bash
cluster_arn="$(aws cloudformation describe-stacks \
  --region us-east-1 \
  --stack-name scope-cloud-runner-production \
  --query "Stacks[0].Outputs[?OutputKey=='RunnerClusterArn'].OutputValue | [0]" \
  --output text)"

aws ecs list-tasks --region us-east-1 --cluster "$cluster_arn"
aws ecs describe-tasks \
  --region us-east-1 \
  --cluster "$cluster_arn" \
  --tasks <task ARN>
```

Check the task's image digest, exit code, stopped reason, and timestamps. Confirm that aborting a Scope run stops its exact ECS task.

For the public mode proof, leave `SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN` unset, run a digest-pinned public image, and confirm that the task definition has no `repositoryCredentials`. For the private mode proof, apply the exact-secret IAM grant, configure the worker ARN, then run a digest-pinned private image. Confirm that the task reaches `RUNNING`, claims its Scope attempt, and references the exact configured ARN. Do not print or fetch the secret value during either proof.

Use IAM simulation after the stack update. The dispatcher must be denied `secretsmanager:GetSecretValue` for both the registry secret and a sample attempt secret. The task execution role must be allowed for the exact registry ARN and the attempt prefix, and denied for an unrelated secret.

## Verify and measure SOCI

The image workflow publishes one build in three forms during the migration experiment: GHCR, an unchanged raw ECR copy, and a converted ECR SOCI v2 image. Its artifact records every digest. Verify the SOCI tag before running it:

```bash
aws ecr batch-get-image \
  --region us-east-1 \
  --repository-name scope-vcs/production/checks \
  --image-ids imageTag=<SOCI tag> \
  --query 'images[0].imageManifest' \
  --output text \
  | jq -e '.manifests[] | select(.artifactType == "application/vnd.amazon.soci.index.v2+json")'
```

Run ten cold tasks for each digest, changing only the pinned image: GHCR, raw ECR, then SOCI ECR. For every task, preserve `createdAt`, `pullStartedAt`, `pullStoppedAt`, and `startedAt` from `aws ecs describe-tasks`. The task must also print the metadata endpoint's snapshotter:

```bash
node -e 'fetch(process.env.ECS_CONTAINER_METADATA_URI_V4).then(r => r.json()).then(m => console.log(JSON.stringify({snapshotter:m.Snapshotter})))'
```

The raw variants should report `overlayfs`; the SOCI variant must report `soci`. Compare median and p95 `startedAt - createdAt`, image-pull duration, and end-to-end execution time. Promote only the converted top-level digest—not the raw image digest or the child SOCI descriptor—when the experiment reaches median startup at or below 45 seconds, p95 at or below 60 seconds, and execution time within 5% of baseline.

Pin the promoted digest in `.scope/runs/checks.yml`, deploy, and observe three healthy production runs. During that hold, the image workflow intentionally keeps GHCR and raw ECR variants available for rollback and measurement. After the hold, remove GHCR publication and keep the last known-good digest as the rollback target. The repository retains tagged artifacts; its lifecycle policy deletes only untagged artifacts older than fourteen days.

## Disable and roll back

Disable cloud execution in the worker before changing infrastructure. Stop any remaining task by ARN:

```bash
aws ecs stop-task \
  --region us-east-1 \
  --cluster "$cluster_arn" \
  --task <task ARN> \
  --reason "Scope Fargate rollback"
```

Queued jobs remain in Scope and can resume after the worker is fixed. Do not delete the stack as a first response because stack deletion also removes the log group and its diagnostic logs.

## Rotate or revoke the dispatcher key

Create a second key, update Railway, verify one run, then remove the old key:

```bash
aws iam list-access-keys --user-name "$dispatcher_user"
aws iam create-access-key --user-name "$dispatcher_user"
aws iam delete-access-key \
  --user-name "$dispatcher_user" \
  --access-key-id <old access key ID>
```

IAM allows at most two keys per user. Revoke both keys immediately if either secret leaves the intended secret store.
