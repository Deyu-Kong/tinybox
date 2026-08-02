use anyhow::{Context, Result};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::unistd::pivot_root;
use std::fs;
use std::path::PathBuf;

pub struct RootfsConfig {
    pub rootfs_path: PathBuf,
    pub work_dir: PathBuf,
}

impl RootfsConfig {
    pub fn new(rootfs_path: PathBuf) -> Result<Self> {
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
        let result = RootfsConfig::new(PathBuf::from("/nonexistent/path"));
        assert!(result.is_err());
    }

    #[test]
    fn test_rootfs_config_new_file() {
        let result = RootfsConfig::new(PathBuf::from("/etc/passwd"));
        assert!(result.is_err());
    }
}
