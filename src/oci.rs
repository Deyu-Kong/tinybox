use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamespaceType {
    Pid,
    Mount,
    Uts,
    Network,
    Ipc,
    Cgroup,
}

impl NamespaceType {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "pid" => Ok(Self::Pid),
            "mount" => Ok(Self::Mount),
            "uts" => Ok(Self::Uts),
            "network" => Ok(Self::Network),
            "ipc" => Ok(Self::Ipc),
            "cgroup" => Ok(Self::Cgroup),
            "user" => anyhow::bail!(
                "OCI user namespace is unsupported until uid/gid mappings are implemented"
            ),
            other => anyhow::bail!("unsupported OCI namespace type: {other}"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OciConfig {
    process: Option<OciProcess>,
    root: Option<OciRoot>,
    linux: Option<OciLinux>,
}

#[derive(Debug, Deserialize)]
struct OciProcess {
    args: Vec<String>,
    #[serde(default)]
    env: Vec<String>,
    cwd: Option<String>,
    user: Option<OciUser>,
}

#[derive(Debug, Deserialize)]
struct OciUser {
    #[serde(default)]
    uid: u32,
    #[serde(default)]
    gid: u32,
}

#[derive(Debug, Deserialize)]
struct OciRoot {
    path: PathBuf,
    #[serde(default)]
    readonly: bool,
}

#[derive(Debug, Deserialize)]
struct OciLinux {
    #[serde(default)]
    namespaces: Vec<OciNamespace>,
}

#[derive(Debug, Deserialize)]
struct OciNamespace {
    #[serde(rename = "type")]
    ns_type: String,
}

/// Which namespaces tinybox should unshare. `None` means "all of them"
/// (tinybox's default, maximum isolation). `Some(set)` means honor only the
/// listed namespace types (OCI semantics).
pub struct OciBundle {
    pub command: Vec<String>,
    pub rootfs: PathBuf,
    pub env: Vec<String>,
    pub root_readonly: bool,
    pub cwd: Option<String>,
    pub uid: u32,
    pub gid: u32,
    pub namespaces: Option<Vec<NamespaceType>>,
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
    let root = config.root.context("OCI config missing root")?;
    let rootfs = if root.path.is_absolute() {
        root.path
    } else {
        bundle.join(root.path)
    };
    if !rootfs.is_dir() {
        anyhow::bail!("OCI rootfs is not a directory: {}", rootfs.display());
    }
    let (uid, gid) = match process.user {
        Some(u) => (u.uid, u.gid),
        None => (0, 0),
    };
    // P1-1: honor linux.namespaces. If the bundle lists any, tinybox unshares
    // only those (OCI semantics); if absent, tinybox keeps its default of all
    // namespaces (maximum isolation).
    let namespaces = match config.linux {
        Some(linux) if !linux.namespaces.is_empty() => Some(
            linux
                .namespaces
                .into_iter()
                .map(|namespace| NamespaceType::parse(&namespace.ns_type))
                .collect::<Result<Vec<_>>>()?,
        ),
        _ => None,
    };
    Ok(OciBundle {
        command: process.args,
        rootfs,
        env: process.env,
        root_readonly: root.readonly,
        cwd: process.cwd,
        uid,
        gid,
        namespaces,
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
        assert!(!bundle.root_readonly);
        assert_eq!(bundle.uid, 0);
        assert_eq!(bundle.gid, 0);
        assert!(bundle.namespaces.is_none()); // absent → default-all
    }

    #[test]
    fn honors_readonly_cwd_user_and_namespaces() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("rootfs")).unwrap();
        let mut file = fs::File::create(dir.path().join("config.json")).unwrap();
        write!(
            file,
            r#"{{"process":{{"args":["sh"],"cwd":"/work","user":{{"uid":1000,"gid":1000}}}},"root":{{"path":"rootfs","readonly":true}},"linux":{{"namespaces":[{{"type":"pid"}},{{"type":"mount"}}]}}}}"#
        )
        .unwrap();
        let bundle = load_bundle(dir.path()).unwrap();
        assert!(bundle.root_readonly);
        assert_eq!(bundle.cwd.as_deref(), Some("/work"));
        assert_eq!(bundle.uid, 1000);
        assert_eq!(bundle.gid, 1000);
        assert_eq!(
            bundle.namespaces,
            Some(vec![NamespaceType::Pid, NamespaceType::Mount])
        );
    }

    #[test]
    fn rejects_unknown_and_user_namespaces() {
        for namespace in ["user", "future"] {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir(dir.path().join("rootfs")).unwrap();
            fs::write(
                dir.path().join("config.json"),
                format!(
                    r#"{{"process":{{"args":["sh"]}},"root":{{"path":"rootfs"}},"linux":{{"namespaces":[{{"type":"{namespace}"}}]}}}}"#
                ),
            )
            .unwrap();
            assert!(load_bundle(dir.path()).is_err());
        }
    }
}
