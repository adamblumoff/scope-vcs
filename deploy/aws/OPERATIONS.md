# Fargate cloud runner operations

CloudFormation owns the runner VPC, public subnets, route to the internet, security group, ECS cluster, log group, task execution role, dispatcher IAM user, and optional budget. Do not create parallel resources in the AWS Console.

The cluster uses Fargate On-Demand. Each task gets a public IPv4 address because the runner must reach GHCR, the Scope API, the cache, and source hosts. The security group has no inbound rules and permits outbound HTTPS only. There is no NAT gateway or idle compute cost.

The worker registers one `scope-runner-<attempt ID>` task definition per attempt because ECS cannot override either the container image or secret references in `RunTask`. The definition contains the digest-pinned image and a reference to a per-attempt Secrets Manager bootstrap credential. The credential value is never placed in the ECS task override or returned by `DescribeTasks`. After ECS reports the task stopped, the worker deregisters the task definition and force-deletes the one-use secret.

Each task is also keyed by its Scope attempt ID. The runtime has an absolute 24-hour watchdog, independent of the worker and heartbeat lease. The worker also hard-expires the database attempt and waits for ECS to report the old task as `STOPPED` before the job can dispatch another attempt. If `RunTask` succeeds but its response is lost, the worker polls for the task by attempt ID for five minutes, using bounded exponential backoff for ECS eventual consistency, before concluding no task was created. Cleanup claims run concurrently and are held for fifteen minutes so another worker cannot race that reconciliation.

## Prerequisites

- AWS CLI v2 authenticated to the production account
- permission to manage CloudFormation, VPC, ECS, IAM, CloudWatch Logs, and AWS Budgets
- Railway CLI authenticated to the Scope production project when setting worker variables

Check the AWS identity before making a plan:

```bash
aws sts get-caller-identity
```

Never deploy this stack from the AWS root user. Use an administrative role with MFA for the initial stack and a CI deployment role for later changes.

Before deploying the application cutover, stop dispatch and drain every nonterminal Northflank attempt. The maintenance migration deliberately refuses to remove provider identity while any attempt is still dispatching or running. Apply the migration only after this query returns zero:

```sql
SELECT count(*)
FROM scope_run_attempts
WHERE state NOT IN ('succeeded', 'failed', 'canceled', 'lost');
```

## Validate and preview

The script defaults to `us-east-1`, stack `scope-cloud-runner-production`, project `scope-vcs`, and environment `production`.

```bash
deploy/aws/apply-cloud-runner.sh validate
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

Copy the returned access key ID and secret directly into Railway. Do not save the JSON to the repository or shell history. The worker also needs the six non-secret stack outputs:

```text
AWS_REGION                    <- AwsRegion
SCOPE_ECS_CLUSTER_ARN         <- RunnerClusterArn
SCOPE_ECS_SUBNET_IDS          <- RunnerSubnetIds
SCOPE_ECS_SECURITY_GROUP_ID   <- RunnerSecurityGroupId
SCOPE_ECS_EXECUTION_ROLE_ARN  <- RunnerExecutionRoleArn
SCOPE_ECS_LOG_GROUP           <- RunnerLogGroupName
```

Set them with `railway variable set` in the production worker service. Set `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` in the same command or through a secure, non-recorded shell session. Do not paste secrets into command examples, tickets, or logs.

The dispatcher's policy permits only these operations:

- register, list, deregister, and tag `scope-runner-attempt_*` task definitions
- run those task definitions only in this cluster
- find an ambiguously started task by its Scope attempt ID
- describe, stop, and tag tasks in this cluster
- create and delete project-tagged per-attempt bootstrap secrets without permission to read them
- pass the exact task execution role to ECS

The task execution role can read only secrets under this cluster's per-attempt prefix so the ECS agent can inject the bootstrap value. Those credentials are not available inside the container. The tasks receive no task IAM role, and runner code has no AWS credentials.

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

Check the task's image digest, exit code, stopped reason, and timestamps. Confirm that aborting a Scope run stops its exact ECS task. Keep Northflank available until a real run and cancellation both pass.

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
