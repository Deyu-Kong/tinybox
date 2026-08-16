use anyhow::{Context, Result};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::unistd::pivot_root;
use std::fs;
use std::path::PathBuf;

pub struct RootfsConfig {
    pub rootfs_path: PathBuf,
    pub work_dir: PathBuf,
    pub readonly: bool,
}

impl RootfsConfig {
    pub fn new(rootfs_path: PathBuf, readonly: bool) -> Result<Self> {
        if !rootfs_path.exists() {
            anyhow::bail!("rootfs path does not exist: {:?}", rootfs_path);
        }
        if !rootfs_path.is_dir() {
            anyhow::bail!("rootfs path is not a directory: {:?}", rootfs_path);
        }

        let work_dir = std::env::temp_dir().join(format!("tinybox-{}", std::process::id()));
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

        // P2-1: honor OCI root.readonly by making the rootfs overlay read-only.
        // /dev, /tmp, /sys, /proc are separate mountpoints set up below, so
        // they remain writable regardless of the root ro flag.
        if self.readonly {
            mount(
                None::<&str>,
                &merged,
                None::<&str>,
                MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
                None::<&str>,
            )
            .context("failed to remount overlay read-only")?;
        }

        Ok(())
    }

    /// Mount /dev, /tmp, /sys, /proc under `merged` BEFORE pivot_root, so the
    /// host's /dev/* device nodes are still reachable as bind sources and the
    /// mounts are carried into the new root by pivot_root. P2-1.
    pub fn setup_special_fs(&self) -> Result<()> {
        let merged = self.work_dir.join("merged");

        // /proc
        let proc_dir = merged.join("proc");
        fs::create_dir_all(&proc_dir).ok();
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
            fs::File::create(&tgt).ok();
            mount(
                Some(host.as_str()),
                &tgt,
                None::<&str>,
                MsFlags::MS_BIND,
                None::<&str>,
            )
            .ok();
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
        .ok();
        // /dev/ptmx → /dev/pts/ptmx
        let ptmx = dev_dir.join("ptmx");
        let _ = std::os::unix::fs::symlink("pts/ptmx", &ptmx);

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
        .ok();

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
        .ok();

        Ok(())
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

        Ok(())
    }

    pub fn cleanup(&self) {
        let _ = umount2(&self.work_dir.join("merged"), MntFlags::MNT_DETACH);
        let _ = fs::remove_dir_all(&self.work_dir);
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
}
