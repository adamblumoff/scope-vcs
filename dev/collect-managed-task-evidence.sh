#!/usr/bin/env bash
set -euo pipefail

cluster_arn="${1:?usage: collect-managed-task-evidence.sh CLUSTER_ARN TASK_ARN OUTPUT_JSON}"
task_arn="${2:?usage: collect-managed-task-evidence.sh CLUSTER_ARN TASK_ARN OUTPUT_JSON}"
output_path="${3:?usage: collect-managed-task-evidence.sh CLUSTER_ARN TASK_ARN OUTPUT_JSON}"
aws_region="${AWS_REGION:-us-east-1}"

if [[ -e "$output_path" ]]; then
  echo "refusing to overwrite $output_path" >&2
  exit 1
fi

umask 077
work_dir=$(mktemp -d)
cleanup() {
  rm -rf -- "$work_dir"
}
trap cleanup EXIT

observed_at=$(date -u +%FT%TZ)
container_instance_arn=""
deadline=$((SECONDS + 3600))

while (( SECONDS < deadline )); do
  aws ecs describe-tasks \
    --region "$aws_region" \
    --cluster "$cluster_arn" \
    --tasks "$task_arn" \
    --include TAGS > "$work_dir/task.json"
  container_instance_arn=$(jq -r '.tasks[0].containerInstanceArn // empty' "$work_dir/task.json")
  [[ -n "$container_instance_arn" ]] && break
  sleep 2
done

if [[ -z "$container_instance_arn" ]]; then
  echo "task did not receive a container instance within one hour" >&2
  exit 1
fi

aws ecs describe-container-instances \
  --region "$aws_region" \
  --cluster "$cluster_arn" \
  --container-instances "$container_instance_arn" \
  --include TAGS > "$work_dir/container-instance.json"
ec2_instance_id=$(jq -er '.containerInstances[0].ec2InstanceId' "$work_dir/container-instance.json")
aws ec2 describe-instances \
  --region "$aws_region" \
  --instance-ids "$ec2_instance_id" > "$work_dir/ec2-instance.json"

while (( SECONDS < deadline )); do
  aws ecs describe-tasks \
    --region "$aws_region" \
    --cluster "$cluster_arn" \
    --tasks "$task_arn" \
    --include TAGS > "$work_dir/task.json"
  [[ "$(jq -r '.tasks[0].lastStatus // empty' "$work_dir/task.json")" == "STOPPED" ]] && break
  sleep 5
done

if [[ "$(jq -r '.tasks[0].lastStatus // empty' "$work_dir/task.json")" != "STOPPED" ]]; then
  echo "task did not stop within one hour" >&2
  exit 1
fi

jq -n \
  --arg observedAt "$observed_at" \
  --argjson task "$(<"$work_dir/task.json")" \
  --argjson containerInstance "$(<"$work_dir/container-instance.json")" \
  --argjson ec2Instance "$(<"$work_dir/ec2-instance.json")" \
  '{
    observedAt: $observedAt,
    task: $task.tasks[0],
    taskFailures: $task.failures,
    containerInstance: $containerInstance.containerInstances[0],
    containerInstanceFailures: $containerInstance.failures,
    ec2Instance: $ec2Instance.Reservations[0].Instances[0]
  }' > "$output_path"
echo "$output_path"
