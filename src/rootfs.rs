use anyhow::{Context, Result};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::unistd::pivot_root;
use std::fs;
use std::path::{Path, PathBuf};

pub struct RootfsConfig {
    pub rootfs_path: PathBuf,
    pub work_dir: PathBuf,
    pub readonly: bool,
}

impl RootfsConfig {
    pub fn new(rootfs_path: PathBuf, readonly: bool) -> Result<Self> {
        let work_dir = std::env::temp_dir().join(format!("tinybox-{}", std::process::id()));
        Self::with_work_dir(rootfs_path, readonly, work_dir)
    }

    pub fn with_work_dir(rootfs_path: PathBuf, readonly: bool, work_dir: PathBuf) -> Result<Self> {
        if !rootfs_path.exists() {
            anyhow::bail!("rootfs path does not exist: {:?}", rootfs_path);
        }
        if !rootfs_path.is_dir() {
            anyhow::bail!("rootfs path is not a directory: {:?}", rootfs_path);
        }

        Ok(Self {
            rootfs_path,
            work_dir,
            readonly,
        })
    }

    pub fn setup(&self) -> Result<()> {
        let upperdir = self.work_dir.join("upper");
        let workdir = self.work_dir.join("work");
        let merged = self.work_dir.join("merged");

        fs::create_dir_all(&upperdir).context("failed to create upperdir")?;
        fs::create_dir_all(&workdir).context("failed to create workdir")?;
        fs::create_dir_all(&merged).context("failed to create merged")?;

        let options = format!(
            "lowerdir={},upperdir={},workdir={}",
            self.rootfs_path.display(),
            upperdir.display(),
            workdir.display()
        );

        mount(
            Some("overlay"),
            &merged,
            Some("overlay"),
            MsFlags::empty(),
            Some(options.as_str()),
        )
        .context("failed to mount overlayfs")?;

        // NOTE: the read-only remount is deferred to `pivot()` — doing it
        // here would make `merged` read-only before `pivot()` creates
        // `old_root` inside it.

        Ok(())
    }

    /// Mount /dev, /tmp, /sys, /proc under `merged` BEFORE pivot_root, so the
    /// host's /dev/* device nodes are still reachable as bind sources and the
    /// mounts are carried into the new root by pivot_root. P2-1.
    pub fn setup_special_fs(&self) -> Result<()> {
        let merged = self.work_dir.join("merged");

        // /proc
        let proc_dir = merged.join("proc");
        fs::create_dir_all(&proc_dir).context("failed to create /proc")?;
        mount(
            Some("proc"),
            &proc_dir,
            Some("proc"),
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
            None::<&str>,
        )
        .context("failed to mount /proc")?;

        // /dev (tmpfs)
        let dev_dir = merged.join("dev");
        fs::create_dir_all(&dev_dir).context("failed to create /dev")?;
        mount(
            Some("tmpfs"),
            &dev_dir,
            Some("tmpfs"),
            MsFlags::MS_NOSUID,
            Some("mode=755"),
        )
        .context("failed to mount /dev tmpfs")?;

        // Device nodes — bind from the host (still reachable in our private
        // mount copy before pivot). Create a regular file as the mountpoint.
        for node in ["null", "zero", "urandom", "random", "tty", "full"] {
            let host = format!("/dev/{node}");
            let tgt = dev_dir.join(node);
            fs::File::create(&tgt)
                .with_context(|| format!("failed to create device mountpoint {tgt:?}"))?;
            mount(
                Some(host.as_str()),
                &tgt,
                None::<&str>,
                MsFlags::MS_BIND,
                None::<&str>,
            )
            .with_context(|| format!("failed to bind {host} to {tgt:?}"))?;
        }

        // /dev/pts (devpts, new instance)
        let pts_dir = dev_dir.join("pts");
        fs::create_dir_all(&pts_dir)?;
        mount(
            Some("devpts"),
            &pts_dir,
            Some("devpts"),
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
            Some("newinstance,ptmxmode=0666"),
        )
        .context("failed to mount private devpts")?;
        // /dev/ptmx → /dev/pts/ptmx
        let ptmx = dev_dir.join("ptmx");
        std::os::unix::fs::symlink("pts/ptmx", &ptmx)
            .context("failed to create /dev/ptmx symlink")?;

        // /dev/shm (tmpfs, 1777)
        let shm_dir = dev_dir.join("shm");
        fs::create_dir_all(&shm_dir)?;
        mount(
            Some("tmpfs"),
            &shm_dir,
            Some("tmpfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
            Some("mode=1777"),
        )
        .context("failed to mount /dev/shm")?;

        // /tmp (tmpfs, size-capped)
        let tmp_dir = merged.join("tmp");
        fs::create_dir_all(&tmp_dir)?;
        mount(
            Some("tmpfs"),
            &tmp_dir,
            Some("tmpfs"),
            MsFlags::MS_NOSUID,
            Some("size=64m"),
        )
        .context("failed to mount /tmp tmpfs")?;

        // /sys — empty read-only tmpfs to avoid leaking host sysfs (PCI/CPU
        // info etc.). A real sysfs bind would expose host hardware; an empty
        // tmpfs is the hardened choice. Programs needing /sys contents will
        // fail — acceptable for an untrusted-code sandbox.
        let sys_dir = merged.join("sys");
        fs::create_dir_all(&sys_dir)?;
        mount(
            Some("tmpfs"),
            &sys_dir,
            Some("tmpfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_RDONLY,
            None::<&str>,
        )
        .context("failed to mount empty read-only /sys")?;

        Ok(())
    }

    pub fn merged_path(&self) -> PathBuf {
        self.work_dir.join("merged")
    }

    pub fn pivot(&self) -> Result<()> {
        let merged = self.work_dir.join("merged");
        let old_root = merged.join("old_root");

        fs::create_dir_all(&old_root).context("failed to create old_root directory")?;

        mount(
            Some(&merged),
            &merged,
            None::<&str>,
            MsFlags::MS_BIND | MsFlags::MS_REC,
            None::<&str>,
        )
        .context("failed to bind mount merged")?;

        pivot_root(&merged, &old_root).context("failed to pivot_root")?;

        std::env::set_current_dir("/").context("failed to chdir to /")?;

        umount2("/old_root", MntFlags::MNT_DETACH).context("failed to umount old_root")?;
        fs::remove_dir("/old_root").ok();

        // P2-1: honor OCI root.readonly — remount the new root read-only AFTER
        // pivot. /dev, /tmp, /sys, /proc are separate mountpoints (set up in
        // setup_special_fs) and remain writable regardless.
        if self.readonly {
            mount(
                None::<&str>,
                "/",
                None::<&str>,
                MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
                None::<&str>,
            )
            .context("failed to remount root read-only")?;
        }

        Ok(())
    }

    pub fn cleanup(&self) {
        let _ = umount2(&self.work_dir.join("merged"), MntFlags::MNT_DETACH);
        let _ = fs::remove_dir_all(&self.work_dir);
    }
}

pub fn mount_volumes(volumes: &[String], target_root: &Path) -> Result<()> {
    for volume in volumes {
        let (host, container, readonly) = parse_volume(volume)?;
        let host = Path::new(host);
        if !host.exists() {
            anyhow::bail!("volume source does not exist: {}", host.display());
        }
        let host = fs::canonicalize(host)
            .with_context(|| format!("failed to resolve volume source {}", host.display()))?;

        let relative = Path::new(container)
            .strip_prefix("/")
            .context("volume target must be an absolute path")?;
        if relative.as_os_str().is_empty() {
            anyhow::bail!("mounting a volume over the sandbox root is forbidden");
        }
        if relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::CurDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            anyhow::bail!("volume target must be a normalized absolute path: {container}");
        }
        reject_symlink_components(target_root, relative)?;
        let target = target_root.join(relative);
        if host.is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("failed to create volume target {}", target.display()))?;
        } else if host.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create volume target parent {}", parent.display())
                })?;
            }
            fs::File::create(&target).with_context(|| {
                format!("failed to create volume file target {}", target.display())
            })?;
        } else {
            anyhow::bail!("volume source must be a regular file or directory: {host:?}");
        }

        mount(
            Some(&host),
            &target,
            None::<&str>,
            MsFlags::MS_BIND | MsFlags::MS_REC,
            None::<&str>,
        )
        .with_context(|| format!("failed to bind {} to {}", host.display(), target.display()))?;

        if readonly {
            mount(
                None::<&str>,
                &target,
                None::<&str>,
                MsFlags::MS_REMOUNT
                    | MsFlags::MS_BIND
                    | MsFlags::MS_RDONLY
                    | MsFlags::MS_NOSUID
                    | MsFlags::MS_NODEV,
                None::<&str>,
            )
            .with_context(|| format!("failed to remount {} read-only", target.display()))?;
        }
    }
    Ok(())
}

fn reject_symlink_components(root: &Path, relative: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!(
                    "volume target traverses a symbolic link: {}",
                    current.display()
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect volume target {}", current.display())
                });
            }
        }
    }
    Ok(())
}

fn parse_volume(value: &str) -> Result<(&str, &str, bool)> {
    let parts: Vec<&str> = value.split(':').collect();
    match parts.as_slice() {
        [host, container] => Ok((host, container, false)),
        [host, container, "ro"] => Ok((host, container, true)),
        _ => anyhow::bail!("invalid volume spec: {value}; expected HOST:CONTAINER[:ro]"),
    }
}

impl Drop for RootfsConfig {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rootfs_config_new_nonexistent() {
        let result = RootfsConfig::new(PathBuf::from("/nonexistent/path"), false);
        assert!(result.is_err());
    }

    #[test]
    fn test_rootfs_config_new_file() {
        let result = RootfsConfig::new(PathBuf::from("/etc/passwd"), false);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_volume_specs_before_mounting() {
        assert!(parse_volume("missing-target").is_err());
        assert!(parse_volume("/host:relative").is_ok());
        assert!(parse_volume("/host:/target:rw").is_err());
    }

    #[test]
    fn rejects_symlink_in_volume_target() {
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/tmp", root.path().join("escape")).unwrap();
        assert!(reject_symlink_components(root.path(), Path::new("escape/data")).is_err());
    }
}
