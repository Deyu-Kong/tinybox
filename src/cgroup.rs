use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub struct CgroupConfig {
    pub name: String,
    pub mem_limit: Option<u64>,
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

        fs::create_dir_all(&path).context("failed to create cgroup directory")?;

        if let Some(mem_limit) = config.mem_limit {
            let mem_max_path = path.join("memory.max");
            fs::write(&mem_max_path, mem_limit.to_string())
                .with_context(|| format!("failed to write memory.max to {:?}", mem_max_path))?;

            let swap_max_path = path.join("memory.swap.max");
            fs::write(&swap_max_path, "0").ok();
        }

        Ok(Self { path })
    }

    pub fn add_process(&self, pid: u32) -> Result<()> {
        let procs_path = self.path.join("cgroup.procs");
        fs::write(&procs_path, pid.to_string())
            .with_context(|| format!("failed to add pid {} to cgroup", pid))?;
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

pub fn parse_mem_limit(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("empty memory limit");
    }

    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('G') {
        (n, 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('M') {
        (n, 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('K') {
        (n, 1024)
    } else if let Some(n) = s.strip_suffix("GB") {
        (n, 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix("MB") {
        (n, 1024 * 1024)
    } else if let Some(n) = s.strip_suffix("KB") {
        (n, 1024)
    } else {
        (s, 1)
    };

    let num: u64 = num_str
        .parse()
        .with_context(|| format!("invalid memory limit: {}", s))?;

    Ok(num * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mem_limit_bytes() {
        assert_eq!(parse_mem_limit("1024").unwrap(), 1024);
    }

    #[test]
    fn test_parse_mem_limit_k() {
        assert_eq!(parse_mem_limit("1K").unwrap(), 1024);
        assert_eq!(parse_mem_limit("1KB").unwrap(), 1024);
    }

    #[test]
    fn test_parse_mem_limit_m() {
        assert_eq!(parse_mem_limit("64M").unwrap(), 64 * 1024 * 1024);
        assert_eq!(parse_mem_limit("64MB").unwrap(), 64 * 1024 * 1024);
    }

    #[test]
    fn test_parse_mem_limit_g() {
        assert_eq!(parse_mem_limit("1G").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_mem_limit("1GB").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_mem_limit_invalid() {
        assert!(parse_mem_limit("").is_err());
        assert!(parse_mem_limit("abc").is_err());
        assert!(parse_mem_limit("64X").is_err());
    }
}
