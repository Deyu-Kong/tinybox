use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct OciConfig {
    process: Option<OciProcess>,
    root: Option<OciRoot>,
}

#[derive(Debug, Deserialize)]
struct OciProcess {
    args: Vec<String>,
    #[serde(default)]
    env: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OciRoot {
    path: PathBuf,
}

pub struct OciBundle {
    pub command: Vec<String>,
    pub rootfs: PathBuf,
    pub env: Vec<String>,
}

pub fn load_bundle(bundle: &Path) -> Result<OciBundle> {
    let config_path = bundle.join("config.json");
    let contents = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read OCI config: {}", config_path.display()))?;
    let config: OciConfig = serde_json::from_str(&contents).context("invalid OCI config.json")?;
    let process = config.process.context("OCI config missing process")?;
    if process.args.is_empty() {
        anyhow::bail!("OCI process.args must not be empty");
    }
    let root_path = config.root.context("OCI config missing root")?.path;
    let rootfs = if root_path.is_absolute() {
        root_path
    } else {
        bundle.join(root_path)
    };
    if !rootfs.is_dir() {
        anyhow::bail!("OCI rootfs is not a directory: {}", rootfs.display());
    }
    Ok(OciBundle {
        command: process.args,
        rootfs,
        env: process.env,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn loads_relative_rootfs_and_process() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("rootfs")).unwrap();
        let mut file = fs::File::create(dir.path().join("config.json")).unwrap();
        write!(
            file,
            r#"{{"process":{{"args":["sh"],"env":["A=B"]}},"root":{{"path":"rootfs"}}}}"#
        )
        .unwrap();
        let bundle = load_bundle(dir.path()).unwrap();
        assert_eq!(bundle.command, vec!["sh"]);
        assert_eq!(bundle.env, vec!["A=B"]);
        assert_eq!(bundle.rootfs, dir.path().join("rootfs"));
    }
}
