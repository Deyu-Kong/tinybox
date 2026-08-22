use anyhow::{bail, Result};
use serde::Serialize;
use std::fs;

#[derive(Serialize)]
struct Check {
    name: &'static str,
    status: &'static str,
    detail: String,
}

pub fn run(json: bool) -> Result<()> {
    let release = fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_else(|_| "unknown".into())
        .trim()
        .to_string();
    let controllers = fs::read_to_string("/sys/fs/cgroup/cgroup.controllers").unwrap_or_default();
    let filesystems = fs::read_to_string("/proc/filesystems").unwrap_or_default();
    let landlock = crate::landlock::abi_version();
    let mut checks = vec![
        check("linux", cfg!(target_os = "linux"), std::env::consts::OS),
        check("kernel", kernel_at_least(&release, 5, 10), &release),
        check(
            "cgroup_v2",
            ["cpu", "memory", "pids"]
                .iter()
                .all(|name| controllers.split_whitespace().any(|value| value == *name)),
            controllers.trim(),
        ),
        check(
            "overlayfs",
            filesystems.lines().any(|line| line.ends_with("overlay")),
            "overlay filesystem",
        ),
        check(
            "landlock",
            landlock.is_some(),
            &landlock.map_or_else(|| "unavailable".into(), |abi| format!("ABI {abi}")),
        ),
    ];
    checks.push(Check {
        name: "privilege",
        status: if unsafe { libc::geteuid() } == 0 {
            "pass"
        } else {
            "warn"
        },
        detail: if unsafe { libc::geteuid() } == 0 {
            "running as root".into()
        } else {
            "runtime operations require a root daemon".into()
        },
    });
    if json {
        println!("{}", serde_json::to_string(&checks)?);
    } else {
        for item in &checks {
            println!(
                "{:<5} {:<12} {}",
                item.status.to_uppercase(),
                item.name,
                item.detail
            );
        }
    }
    if checks.iter().any(|item| item.status == "fail") {
        bail!("host does not satisfy tinybox runtime requirements");
    }
    Ok(())
}

fn check(name: &'static str, passed: bool, detail: &str) -> Check {
    Check {
        name,
        status: if passed { "pass" } else { "fail" },
        detail: detail.to_string(),
    }
}

fn kernel_at_least(release: &str, major: u64, minor: u64) -> bool {
    let mut parts = release
        .split(['.', '-'])
        .filter_map(|part| part.parse::<u64>().ok());
    matches!((parts.next(), parts.next()), (Some(found_major), Some(found_minor)) if (found_major, found_minor) >= (major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kernel_baseline() {
        assert!(kernel_at_least("5.10.0-generic", 5, 10));
        assert!(kernel_at_least("6.8.12", 5, 10));
        assert!(!kernel_at_least("5.4.0", 5, 10));
        assert!(!kernel_at_least("invalid", 5, 10));
    }
}
