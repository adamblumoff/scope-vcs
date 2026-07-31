use anyhow::{Context, bail};
use std::{
    fs,
    num::NonZeroUsize,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    process::Command,
};

const MIB: u64 = 1024 * 1024;
const DAEMON_MEMORY_RESERVE: u64 = 512 * MIB;
const MIN_JOB_MEMORY: u64 = 512 * MIB;
const PID_RESERVE: u64 = 64;
const MIN_JOB_PIDS: u64 = 128;
const MAX_JOB_PIDS: u64 = 4096;
const DAEMON_CPU_RESERVE_MILLIS: u64 = 500;
const MIN_JOB_CPU_MILLIS: u64 = 500;
const TRANSIENT_DISK_RESERVE: u64 = 5 * 1024 * MIB;
const MIN_JOB_STORAGE: u64 = 2 * 1024 * MIB;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResourceLimits {
    pub(super) memory_bytes: u64,
    pub(super) cpu_millis: u64,
    pub(super) pids: u64,
    pub(super) storage_bytes: u64,
}

impl ResourceLimits {
    pub(super) fn detect() -> anyhow::Result<Self> {
        let host_memory = mem_available_bytes(&fs::read_to_string("/proc/meminfo")?)?;
        let cgroup_path = unified_cgroup_path(&fs::read_to_string("/proc/self/cgroup")?)?;
        let cgroup = read_cgroup_capacity(Path::new("/sys/fs/cgroup"), &cgroup_path)?;
        let memory_headroom = cgroup
            .memory_headroom
            .map_or(host_memory, |value| value.min(host_memory));
        if memory_headroom < DAEMON_MEMORY_RESERVE + MIN_JOB_MEMORY {
            bail!(
                "runner has {} MiB memory headroom; at least {} MiB is required",
                memory_headroom / MIB,
                (DAEMON_MEMORY_RESERVE + MIN_JOB_MEMORY) / MIB
            );
        }

        let host_cpus = std::thread::available_parallelism()
            .unwrap_or(NonZeroUsize::new(1).expect("one is non-zero"))
            .get() as u64
            * 1000;
        let cpu_capacity = [
            Some(host_cpus),
            cgroup.cpu_quota_millis,
            cgroup.cpuset_cpus.map(|count| count * 1000),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(1000);
        let cpu_millis = job_cpu_millis(cpu_capacity)?;

        let pids = cgroup
            .pid_headroom
            .map(|headroom| headroom.saturating_sub(PID_RESERVE))
            .unwrap_or(MAX_JOB_PIDS)
            .min(MAX_JOB_PIDS);
        if pids < MIN_JOB_PIDS {
            bail!("runner has {pids} available PIDs; at least {MIN_JOB_PIDS} are required");
        }
        let storage_bytes = transient_storage_bytes()?;

        Ok(Self {
            memory_bytes: memory_headroom - DAEMON_MEMORY_RESERVE,
            cpu_millis,
            pids,
            storage_bytes,
        })
    }

    pub(super) fn apply(&self, command: &mut Command) {
        let cpus = format!("{}.{:03}", self.cpu_millis / 1000, self.cpu_millis % 1000);
        let memory = self.memory_bytes.to_string();
        let pids = self.pids.to_string();
        command.args([
            "--memory",
            &memory,
            "--memory-swap",
            &memory,
            "--cpus",
            &cpus,
            "--pids-limit",
            &pids,
        ]);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CgroupCapacity {
    memory_headroom: Option<u64>,
    pid_headroom: Option<u64>,
    cpu_quota_millis: Option<u64>,
    cpuset_cpus: Option<u64>,
}

fn unified_cgroup_path(cgroup: &str) -> anyhow::Result<PathBuf> {
    let path = cgroup
        .lines()
        .find_map(|line| {
            let mut fields = line.splitn(3, ':');
            match (fields.next(), fields.next(), fields.next()) {
                (Some("0"), Some(""), Some(path)) => Some(path),
                _ => None,
            }
        })
        .context("process is not attached to a unified cgroup v2 hierarchy")?;
    let path = Path::new(path);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        bail!("unified process cgroup path is invalid");
    }
    Ok(path
        .strip_prefix("/")
        .expect("validated cgroup paths are absolute")
        .to_path_buf())
}

fn read_cgroup_capacity(root: &Path, relative_path: &Path) -> anyhow::Result<CgroupCapacity> {
    let leaf = root.join(relative_path);
    if !leaf.is_dir() {
        bail!(
            "current unified cgroup is unavailable at {}",
            leaf.display()
        );
    }
    let mut capacity = CgroupCapacity::default();
    for directory in cgroup_ancestors(root, &leaf)? {
        capacity.memory_headroom = minimum(
            capacity.memory_headroom,
            optional_headroom(&directory, "memory.max", "memory.current")?,
        );
        capacity.pid_headroom = minimum(
            capacity.pid_headroom,
            optional_headroom(&directory, "pids.max", "pids.current")?,
        );
        if let Some(cpu_max) = optional_file(&directory.join("cpu.max"))? {
            capacity.cpu_quota_millis =
                minimum(capacity.cpu_quota_millis, cpu_quota_millis(&cpu_max)?);
        }
        if let Some(cpuset) = optional_file(&directory.join("cpuset.cpus.effective"))? {
            capacity.cpuset_cpus = minimum(capacity.cpuset_cpus, Some(cpuset_count(&cpuset)?));
        }
    }
    Ok(capacity)
}

fn cgroup_ancestors(root: &Path, leaf: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if !leaf.starts_with(root) {
        bail!("current cgroup escapes the unified hierarchy");
    }
    let mut directories = Vec::new();
    let mut current = leaf.to_path_buf();
    loop {
        directories.push(current.clone());
        if current == root {
            break;
        }
        if !current.pop() {
            bail!("current cgroup has no unified hierarchy ancestor");
        }
    }
    Ok(directories)
}

fn optional_headroom(
    directory: &Path,
    max_name: &str,
    current_name: &str,
) -> anyhow::Result<Option<u64>> {
    let max_path = directory.join(max_name);
    let current_path = directory.join(current_name);
    let max = optional_file(&max_path)?;
    let current = optional_file(&current_path)?;
    match (max, current) {
        (None, None) => Ok(None),
        (Some(max), Some(_)) if max.trim() == "max" => Ok(None),
        (Some(max), Some(current)) => {
            let max = max
                .trim()
                .parse::<u64>()
                .with_context(|| format!("parse {}", max_path.display()))?;
            let current = current
                .trim()
                .parse::<u64>()
                .with_context(|| format!("parse {}", current_path.display()))?;
            Ok(Some(max.saturating_sub(current)))
        }
        _ => bail!(
            "cgroup controller files are incomplete at {}",
            directory.display()
        ),
    }
}

fn optional_file(path: &Path) -> anyhow::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn minimum(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn job_cpu_millis(capacity: u64) -> anyhow::Result<u64> {
    let required = DAEMON_CPU_RESERVE_MILLIS + MIN_JOB_CPU_MILLIS;
    if capacity < required {
        bail!(
            "runner has {:.3} CPU capacity; at least {:.3} CPU is required to preserve the daemon reserve",
            capacity as f64 / 1000.0,
            required as f64 / 1000.0
        );
    }
    Ok(capacity - DAEMON_CPU_RESERVE_MILLIS)
}

fn transient_storage_bytes() -> anyhow::Result<u64> {
    let output = Command::new("docker")
        .args(["info", "--format={{.DockerRootDir}}"])
        .output()
        .context("inspect Docker data root")?;
    if !output.status.success() {
        bail!(
            "inspect Docker data root: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let root = Path::new(&root);
    let root_device = fs::metadata(root)
        .with_context(|| format!("inspect Docker data root {}", root.display()))?
        .dev();
    if root_device == fs::metadata("/")?.dev() {
        bail!("Docker data root must use a dedicated transient filesystem");
    }
    let output = Command::new("df")
        .args(["-B1", "--output=avail"])
        .arg(root)
        .output()
        .context("inspect transient filesystem capacity")?;
    if !output.status.success() {
        bail!(
            "inspect transient filesystem capacity: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let available = String::from_utf8_lossy(&output.stdout)
        .lines()
        .last()
        .context("transient capacity output is empty")?
        .trim()
        .parse::<u64>()
        .context("parse transient filesystem capacity")?;
    let job = available.saturating_sub(TRANSIENT_DISK_RESERVE);
    if job < MIN_JOB_STORAGE {
        bail!(
            "Docker data root has {} MiB safe headroom; at least {} MiB is required",
            job / MIB,
            MIN_JOB_STORAGE / MIB
        );
    }
    Ok(job)
}

fn mem_available_bytes(meminfo: &str) -> anyhow::Result<u64> {
    let kib = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|value| value.split_whitespace().next())
        .context("/proc/meminfo does not report MemAvailable")?
        .parse::<u64>()
        .context("parse MemAvailable")?;
    Ok(kib.saturating_mul(1024))
}

fn cpu_quota_millis(cpu_max: &str) -> anyhow::Result<Option<u64>> {
    let mut values = cpu_max.split_whitespace();
    let quota = values.next().context("cgroup cpu.max is empty")?;
    let period = values
        .next()
        .context("cgroup cpu.max has no period")?
        .parse::<u64>()
        .context("parse cgroup CPU period")?;
    if quota == "max" {
        return Ok(None);
    }
    let quota = quota.parse::<u64>().context("parse cgroup CPU quota")?;
    Ok(Some(quota.saturating_mul(1000) / period.max(1)))
}

fn cpuset_count(value: &str) -> anyhow::Result<u64> {
    if value.trim().is_empty() {
        return Ok(0);
    }
    value.trim().split(',').try_fold(0_u64, |count, range| {
        let (start, end) = range
            .split_once('-')
            .map_or((range, range), |(start, end)| (start, end));
        let start = start.parse::<u64>().context("parse cgroup CPU set")?;
        let end = end.parse::<u64>().context("parse cgroup CPU set")?;
        if end < start {
            bail!("cgroup CPU set range is reversed");
        }
        Ok(count.saturating_add(end - start + 1))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_cgroup_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "scope-cgroup-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn controller(directory: &Path, name: &str, value: &str) {
        fs::write(directory.join(name), value).unwrap();
    }

    #[test]
    fn parses_live_resource_inputs() {
        assert_eq!(
            mem_available_bytes("MemTotal: 9 kB\nMemAvailable: 1024 kB\n").unwrap(),
            MIB
        );
        assert_eq!(cpu_quota_millis("150000 100000").unwrap(), Some(1500));
        assert_eq!(cpu_quota_millis("max 100000").unwrap(), None);
        assert_eq!(cpuset_count("0-2,5,8-9").unwrap(), 6);
        assert_eq!(cpuset_count("").unwrap(), 0);
    }

    #[test]
    fn parses_only_the_unified_process_cgroup_path() {
        assert_eq!(
            unified_cgroup_path(
                "9:memory:/legacy\n0::/user.slice/user-1000.slice/app.slice/service.scope\n"
            )
            .unwrap(),
            PathBuf::from("user.slice/user-1000.slice/app.slice/service.scope")
        );
        assert!(unified_cgroup_path("9:memory:/legacy\n").is_err());
        assert!(unified_cgroup_path("0::/user.slice/../escape\n").is_err());
    }

    #[test]
    fn cgroup_tree_uses_the_tightest_available_ancestor_constraints() {
        let root = synthetic_cgroup_root("capacity");
        let user = root.join("user.slice");
        let app = user.join("app.slice");
        let leaf = app.join("runner.service");
        fs::create_dir_all(&leaf).unwrap();

        controller(&root, "memory.max", "16000\n");
        controller(&root, "memory.current", "1000\n");
        controller(&root, "pids.max", "1000\n");
        controller(&root, "pids.current", "100\n");
        controller(&root, "cpu.max", "400000 100000\n");
        controller(&root, "cpuset.cpus.effective", "0-7\n");

        controller(&user, "memory.max", "9000\n");
        controller(&user, "memory.current", "2000\n");
        controller(&app, "cpu.max", "150000 100000\n");
        controller(&app, "cpuset.cpus.effective", "0-3\n");

        controller(&leaf, "memory.max", "max\n");
        controller(&leaf, "memory.current", "500\n");
        controller(&leaf, "pids.max", "300\n");
        controller(&leaf, "pids.current", "120\n");
        controller(&leaf, "cpu.max", "max 100000\n");
        controller(&leaf, "cpuset.cpus.effective", "2-3\n");

        assert_eq!(
            read_cgroup_capacity(&root, Path::new("user.slice/app.slice/runner.service")).unwrap(),
            CgroupCapacity {
                memory_headroom: Some(7000),
                pid_headroom: Some(180),
                cpu_quota_millis: Some(1500),
                cpuset_cpus: Some(2),
            }
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cgroup_tree_rejects_incomplete_controller_state() {
        let root = synthetic_cgroup_root("incomplete");
        controller(&root, "memory.max", "1024\n");
        assert!(read_cgroup_capacity(&root, Path::new("")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cpu_admission_never_oversubscribes_or_consumes_the_daemon_reserve() {
        assert!(job_cpu_millis(500).is_err());
        assert!(job_cpu_millis(999).is_err());
        assert_eq!(job_cpu_millis(1000).unwrap(), 500);
        assert_eq!(job_cpu_millis(1500).unwrap(), 1000);
        assert_eq!(job_cpu_millis(4000).unwrap(), 3500);
    }

    #[test]
    fn resource_limits_disable_swap_without_an_inferred_cgroup_parent() {
        let limits = ResourceLimits {
            memory_bytes: 1024,
            cpu_millis: 2500,
            pids: 300,
            storage_bytes: 2048,
        };
        let mut command = Command::new("docker");
        limits.apply(&mut command);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            [
                "--memory",
                "1024",
                "--memory-swap",
                "1024",
                "--cpus",
                "2.500",
                "--pids-limit",
                "300"
            ]
        );
        assert!(!args.iter().any(|argument| argument == "--cgroup-parent"));
    }
}
