#!/usr/bin/env bash
set -euo pipefail

if (( $# != 6 )); then
  echo "usage: $0 <source image> <raw ECR image> <SOCI ECR image> <ECR repository> <raw tag> <SOCI tag>" >&2
  exit 2
fi

readonly source_image="$1"
readonly raw_image="$2"
readonly soci_image="$3"
readonly repository_name="$4"
readonly raw_tag="$5"
readonly soci_tag="$6"

for required_command in aws jq skopeo soci; do
  command -v "$required_command" >/dev/null || {
    echo "$required_command is required" >&2
    exit 2
  }
done

if [[ "$source_image" != *@* && "$source_image" != *:* ]]; then
  echo "source image must include a tag or digest" >&2
  exit 2
fi
if [[ "$raw_image" != *:"$raw_tag" || "$soci_image" != *:"$soci_tag" ]]; then
  echo "ECR image references and tags disagree" >&2
  exit 2
fi
for image_tag in "$raw_tag" "$soci_tag"; do
  if [[ ! "$image_tag" =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$ ]]; then
    echo "$image_tag is not a valid OCI tag" >&2
    exit 2
  fi
done

work_dir="$(mktemp -d)"
readonly work_dir
trap 'rm -rf -- "$work_dir"' EXIT

readonly source_archive="$work_dir/source.tar"
readonly soci_archive="$work_dir/soci.tar"

skopeo copy "docker://$source_image" "oci-archive:$source_archive"
skopeo copy --all "oci-archive:$source_archive" "docker://$raw_image"
soci convert --standalone --platform linux/amd64 "$source_archive" "$soci_archive"
skopeo copy --all "oci-archive:$soci_archive" "docker://$soci_image"

image_manifest="$(aws ecr batch-get-image \
  --repository-name "$repository_name" \
  --image-ids "imageTag=$soci_tag" \
  --query 'images[0].imageManifest' \
  --output text)"
readonly image_manifest

jq -e '
  .mediaType == "application/vnd.oci.image.index.v1+json" and
  ([
    .manifests[] |
    select(.artifactType == "application/vnd.amazon.soci.index.v2+json")
  ] | length) == 1
' <<<"$image_manifest" >/dev/null
