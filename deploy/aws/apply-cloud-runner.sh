#!/usr/bin/env bash
set -euo pipefail
export AWS_PAGER=""

readonly command_name="${1:-}"
case "$command_name" in
  validate|plan|apply) ;;
  *)
    echo "usage: $0 <validate|plan|apply> [change-set ARN]" >&2
    exit 2
    ;;
esac
readonly requested_change_set_arn="${2:-}"
if [[ "$command_name" != apply && -n "$requested_change_set_arn" ]]; then
  echo "a change-set ARN is accepted only by apply" >&2
  exit 2
fi

command -v aws >/dev/null || { echo "aws CLI is required" >&2; exit 2; }

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly script_dir
readonly template_file="$script_dir/cloud-runner.yaml"
readonly aws_region="${AWS_REGION:-us-east-1}"
readonly stack_name="${STACK_NAME:-scope-cloud-runner-production}"
readonly environment_name="${ENVIRONMENT_NAME:-production}"
readonly budget_email="${BUDGET_NOTIFICATION_EMAIL:-}"
readonly monthly_budget_usd="${MONTHLY_BUDGET_USD:-100}"
readonly github_repository="${GITHUB_REPOSITORY:-scope-vcs/scope-vcs}"
readonly github_repository_id="${GITHUB_REPOSITORY_ID:-1272896256}"
readonly existing_github_oidc_provider_arn="${EXISTING_GITHUB_OIDC_PROVIDER_ARN:-}"

aws_command() {
  aws --region "$aws_region" "$@"
}

print_outputs() {
  local output_key output_value

  echo "CloudFormation outputs:"
  aws_command cloudformation describe-stacks \
    --stack-name "$stack_name" \
    --query 'Stacks[0].Outputs[].[OutputKey,OutputValue]' \
    --output table

  for output_key in \
    AwsRegion \
    RunnerClusterArn \
    RunnerSubnetIds \
    RunnerSecurityGroupId \
    RunnerExecutionRoleArn \
    RunnerLogGroupName \
    RailwayDispatcherUserName \
    ChecksImageRepositoryName \
    ChecksImageRepositoryUri \
    ChecksImagePublisherRoleArn \
    GitHubInfrastructureRoleArn; do
    output_value="$(aws_command cloudformation describe-stacks \
      --stack-name "$stack_name" \
      --query "Stacks[0].Outputs[?OutputKey=='$output_key'].OutputValue | [0]" \
      --output text)"
    printf '%s=%s\n' "$output_key" "$output_value"
  done
}

caller_arn="$(aws_command sts get-caller-identity --query Arn --output text)"
readonly caller_arn
echo "AWS caller: $caller_arn"
if [[ "$command_name" != validate && "$caller_arn" == *":root" ]]; then
  echo "refusing to change infrastructure with AWS root credentials; assume an administrative role with MFA" >&2
  exit 1
fi
aws_command cloudformation validate-template \
  --template-body "file://$template_file" \
  --query Description \
  --output text

if [[ "$command_name" == validate ]]; then
  exit 0
fi

readonly parameters=(
  "ParameterKey=Environment,ParameterValue=$environment_name"
  "ParameterKey=BudgetNotificationEmail,ParameterValue=$budget_email"
  "ParameterKey=MonthlyBudgetUsd,ParameterValue=$monthly_budget_usd"
  "ParameterKey=GitHubRepository,ParameterValue=$github_repository"
  "ParameterKey=GitHubRepositoryId,ParameterValue=$github_repository_id"
  "ParameterKey=ExistingGitHubOidcProviderArn,ParameterValue=$existing_github_oidc_provider_arn"
)
readonly tags=(
  "Key=Project,Value=scope-vcs"
  "Key=Environment,Value=$environment_name"
  "Key=Component,Value=cloud-runner"
  "Key=ManagedBy,Value=cloudformation"
)

stack_operation=CREATE
change_set_arn="$requested_change_set_arn"
if [[ -n "$change_set_arn" ]]; then
  stack_operation="$(aws_command cloudformation describe-change-set \
    --change-set-name "$change_set_arn" \
    --stack-name "$stack_name" \
    --query ChangeSetType \
    --output text)"
else
  stack_status="$(aws_command cloudformation describe-stacks \
    --stack-name "$stack_name" \
    --query 'Stacks[0].StackStatus' \
    --output text 2>/dev/null || true)"
  case "$stack_status" in
    "")
      stack_operation=CREATE
      ;;
    REVIEW_IN_PROGRESS)
      echo "deleting the unexecuted first-deployment preview for $stack_name"
      aws_command cloudformation delete-stack --stack-name "$stack_name"
      aws_command cloudformation wait stack-delete-complete --stack-name "$stack_name"
      stack_operation=CREATE
      ;;
    ROLLBACK_COMPLETE)
      echo "first deployment rolled back; preserving recent stack events before retry"
      aws_command cloudformation describe-stack-events \
        --stack-name "$stack_name" \
        --max-items 20 \
        --output table
      echo "deleting the rolled-back first deployment for $stack_name"
      aws_command cloudformation delete-stack --stack-name "$stack_name"
      aws_command cloudformation wait stack-delete-complete --stack-name "$stack_name"
      stack_operation=CREATE
      ;;
    CREATE_COMPLETE|UPDATE_COMPLETE|UPDATE_ROLLBACK_COMPLETE)
      stack_operation=UPDATE
      ;;
    *)
      echo "stack $stack_name is not deployable while its status is $stack_status" >&2
      exit 1
      ;;
  esac
  change_set_name="cli-$(date -u +%Y%m%dT%H%M%SZ)-$$"
  readonly change_set_name
  change_set_arn="$(aws_command cloudformation create-change-set \
    --stack-name "$stack_name" \
    --change-set-name "$change_set_name" \
    --change-set-type "$stack_operation" \
    --template-body "file://$template_file" \
    --capabilities CAPABILITY_NAMED_IAM \
    --parameters "${parameters[@]}" \
    --tags "${tags[@]}" \
    --description "CLI plan for Scope Fargate runner infrastructure" \
    --query Id \
    --output text)"

  set +e
  aws_command cloudformation wait change-set-create-complete \
    --change-set-name "$change_set_arn" \
    --stack-name "$stack_name"
  wait_status=$?
  set -e

  if (( wait_status != 0 )); then
    status_reason="$(aws_command cloudformation describe-change-set \
      --change-set-name "$change_set_arn" \
      --stack-name "$stack_name" \
      --query StatusReason \
      --output text)"
    readonly status_reason
    if [[ "$status_reason" == *"didn't contain changes"* || "$status_reason" == *"No updates are to be performed"* ]]; then
      echo "No infrastructure changes."
      print_outputs
      exit 0
    fi
    echo "change set failed: $status_reason" >&2
    exit 1
  fi
fi
readonly change_set_arn
readonly stack_operation

echo "Change set: $change_set_arn"
aws_command cloudformation describe-change-set \
  --change-set-name "$change_set_arn" \
  --stack-name "$stack_name" \
  --query 'Changes[].ResourceChange.[Action,LogicalResourceId,ResourceType,Replacement]' \
  --output table

if [[ "$command_name" == plan ]]; then
  echo "The change set was not executed. Apply this exact plan with:"
  printf '%q apply %q\n' "$0" "$change_set_arn"
  exit 0
fi

aws_command cloudformation execute-change-set \
  --change-set-name "$change_set_arn" \
  --stack-name "$stack_name"

if [[ "$stack_operation" == CREATE ]]; then
  aws_command cloudformation wait stack-create-complete --stack-name "$stack_name"
else
  aws_command cloudformation wait stack-update-complete --stack-name "$stack_name"
fi

print_outputs
