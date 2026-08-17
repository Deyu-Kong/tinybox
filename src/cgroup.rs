use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub struct CgroupConfig {
    pub name: String,
    pub memory: Option<u64>,
    pub cpus: Option<f64>,
    pub cpu_quota: Option<i64>,
    pub cpu_period: Option<u64>,
    pub pids_limit: Option<u64>,
}

pub struct Cgroup {
    path: PathBuf,
}

impl Cgroup {
    pub fn new(config: &CgroupConfig) -> Result<Self> {
        let cgroup_root = Path::new("/sys/fs/cgroup");
        let path = cgroup_root.join(&config.name);

        if !cgroup_root.exists() {
            anyhow::bail!("cgroup v2 not mounted at /sys/fs/cgroup");
        }

        let controllers_path = cgroup_root.join("cgroup.controllers");
        let controllers = fs::read_to_string(&controllers_path).with_context(|| {
            format!(
                "cgroup v2 controllers file is unavailable: {}",
                controllers_path.display()
            )
        })?;
        let require_controller = |name: &str, requested: bool| -> Result<()> {
            if requested && !controllers.split_whitespace().any(|value| value == name) {
                anyhow::bail!("required cgroup v2 controller is unavailable: {name}");
            }
            Ok(())
        };
        require_controller("memory", config.memory.is_some())?;
        require_controller("cpu", config.cpus.is_some() || config.cpu_quota.is_some())?;
        require_controller("pids", config.pids_limit.is_some())?;

        fs::create_dir_all(&path).context("failed to create cgroup directory")?;

        if let Some(memory) = config.memory {
            let mem_max_path = path.join("memory.max");
            fs::write(&mem_max_path, memory.to_string())
                .with_context(|| format!("failed to write memory.max to {:?}", mem_max_path))?;

            let swap_max_path = path.join("memory.swap.max");
            fs::write(&swap_max_path, "0").with_context(|| {
                format!("failed to disable swap in {}", swap_max_path.display())
            })?;
        }

        let period = config.cpu_period.unwrap_or(100_000);

        if let Some(quota) = config.cpu_quota {
            if quota <= 0 {
                anyhow::bail!("--cpu-quota must be positive, got {}", quota);
            }
            let cpu_max_path = path.join("cpu.max");
            fs::write(&cpu_max_path, format!("{} {}", quota, period))
                .with_context(|| format!("failed to write cpu.max to {:?}", cpu_max_path))?;
        } else if let Some(cpus) = config.cpus {
            if cpus <= 0.0 {
                anyhow::bail!("--cpus must be positive, got {}", cpus);
            }
            let quota = (cpus * period as f64) as u64;
            let cpu_max_path = path.join("cpu.max");
            fs::write(&cpu_max_path, format!("{} {}", quota, period))
                .with_context(|| format!("failed to write cpu.max to {:?}", cpu_max_path))?;
        }

        if let Some(pids) = config.pids_limit {
            let pids_max_path = path.join("pids.max");
            fs::write(&pids_max_path, pids.to_string())
                .with_context(|| format!("failed to write pids.max to {:?}", pids_max_path))?;
        }

        Ok(Self { path })
    }

    pub fn add_process(&self, pid: u32) -> Result<()> {
        let procs_path = self.path.join("cgroup.procs");
        fs::write(&procs_path, pid.to_string())
            .with_context(|| format!("failed to add pid {} to cgroup", pid))?;
        Ok(())
    }

    pub fn update_resources(path: &Path, memory: u64, cpus: f64, pids: u64) -> Result<()> {
        if !path.is_dir() {
            anyhow::bail!("sandbox cgroup is not ready");
        }
        fs::write(path.join("memory.max"), memory.to_string())
            .context("failed to update memory.max")?;
        fs::write(
            path.join("cpu.max"),
            format!("{} 100000", (cpus * 100_000.0) as u64),
        )
        .context("failed to update cpu.max")?;
        fs::write(path.join("pids.max"), pids.to_string()).context("failed to update pids.max")?;
        Ok(())
    }

    pub fn cleanup(&self) {
        let _ = fs::remove_dir(&self.path);
    }
}

impl Drop for Cgroup {
    fn drop(&mut self) {
        self.cleanup();
    }
}

pub fn parse_memory(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("empty memory value");
    }

    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('g') {
        (n, 1024u64 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 1024u64 * 1024)
    } else if let Some(n) = s.strip_suffix('k') {
        (n, 1024u64)
    } else if let Some(n) = s.strip_suffix('b') {
        (n, 1u64)
    } else {
        (s, 1u64)
    };

    let num: u64 = num_str
        .parse()
        .with_context(|| format!("invalid memory value: {}", s))?;

    Ok(num * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_memory_bytes() {
        assert_eq!(parse_memory("1024").unwrap(), 1024);
    }

    #[test]
    fn test_parse_memory_k() {
        assert_eq!(parse_memory("1k").unwrap(), 1024);
    }

    #[test]
    fn test_parse_memory_m() {
        assert_eq!(parse_memory("64m").unwrap(), 64 * 1024 * 1024);
        assert_eq!(parse_memory("256m").unwrap(), 256 * 1024 * 1024);
    }

    #[test]
    fn test_parse_memory_g() {
        assert_eq!(parse_memory("1g").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_memory_b() {
        assert_eq!(parse_memory("512b").unwrap(), 512);
    }

    #[test]
    fn test_parse_memory_invalid() {
        assert!(parse_memory("").is_err());
        assert!(parse_memory("abc").is_err());
        assert!(parse_memory("64x").is_err());
    }
}
