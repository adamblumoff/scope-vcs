#!/usr/bin/env bash
set -euo pipefail

echo "PROBE_VERSION=1"
echo "PROBE_STARTED_UTC=$(date -u +%FT%TZ)"
echo "PROBE_SHA=$(git rev-parse HEAD)"
echo "FACT uname=$(uname -a)"
echo "FACT nproc=$(nproc)"
echo "FACT online_processors=$(getconf _NPROCESSORS_ONLN)"
rustc --version --verbose
cargo --version --verbose
lscpu
lscpu --extended=CPU,CORE,SOCKET,NODE,ONLINE,MAXMHZ,MINMHZ,MHZ || true
df -hT . /tmp
findmnt -T . || true
findmnt -T /tmp || true

for fact in \
  /sys/fs/cgroup/cpu.max \
  /sys/fs/cgroup/cpu.weight \
  /sys/fs/cgroup/cpuset.cpus.effective \
  /sys/fs/cgroup/cpu/cpu.cfs_quota_us \
  /sys/fs/cgroup/cpu/cpu.cfs_period_us \
  /sys/fs/cgroup/cpuset/cpuset.cpus; do
  if [[ -r "$fact" ]]; then
    echo "FACT $fact=$(<"$fact")"
  fi
done

print_runtime_stats() {
  local phase=$1
  for stat_file in \
    /sys/fs/cgroup/cpu.stat \
    /sys/fs/cgroup/cpu/cpu.stat \
    /proc/pressure/cpu \
    /proc/pressure/memory \
    /proc/pressure/io; do
    if [[ -r "$stat_file" ]]; then
      while IFS= read -r line; do
        echo "RUNTIME_${phase} $stat_file $line"
      done < "$stat_file"
    fi
  done
}

print_runtime_stats START

probe_root=$(mktemp -d)
cleanup() {
  rm -rf -- "$probe_root"
}
trap cleanup EXIT

rustc \
  --edition=2021 \
  -C opt-level=3 \
  -C target-cpu=x86-64 \
  dev/runner-perf-probe.rs \
  -o "$probe_root/runner-perf-probe"
"$probe_root/runner-perf-probe" "$probe_root"

export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_TARGET_DIR="$probe_root/cargo-target"
cargo fetch --locked
compile_started_ns=$(date +%s%N)
cargo test --workspace --features api/test-support --locked --no-run
compile_finished_ns=$(date +%s%N)
awk \
  -v started="$compile_started_ns" \
  -v finished="$compile_finished_ns" \
  'BEGIN { printf "RESULT cold_workspace_compile_seconds=%.3f\n", (finished - started) / 1000000000 }'

print_runtime_stats END
echo "PROBE_FINISHED_UTC=$(date -u +%FT%TZ)"
