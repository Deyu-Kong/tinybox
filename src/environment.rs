use crate::policy::FsAccess;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentRequest {
    #[default]
    Host,
    Rootfs {
        path: PathBuf,
    },
    Profile {
        name: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentVolume {
    pub source: PathBuf,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvironmentManifest {
    pub version: u32,
    pub source: String,
    pub base_rootfs: PathBuf,
    pub workspace: String,
    pub home: String,
    pub cache: String,
    pub mappings: Vec<Mapping>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mapping {
    pub source: PathBuf,
    pub target: String,
    pub mode: String,
}

pub struct PreparedEnvironment {
    pub state_dir: PathBuf,
    pub rootfs: PathBuf,
    pub rootfs_work_dir: PathBuf,
    pub volumes: Vec<String>,
    pub env: Vec<String>,
    pub filesystem_paths: Vec<(PathBuf, FsAccess)>,
}

pub fn prepare(
    id: &str,
    request: &EnvironmentRequest,
    workspace: &Path,
    volumes: &[EnvironmentVolume],
    requested_env: &[String],
) -> Result<PreparedEnvironment> {
    let state_dir = PathBuf::from("/var/lib/tinybox/tasks").join(id);
    if state_dir.exists() {
        bail!("task state already exists: {}", state_dir.display());
    }
    fs::create_dir_all(state_dir.join("rootfs"))?;
    fs::create_dir_all(state_dir.join("home/.cache"))?;

    let result = (|| -> Result<PreparedEnvironment> {
        let (source, rootfs, profile_env, tool_paths) = resolve_source(request)?;
        let mut mappings = vec![
            Mapping {
                source: workspace.to_path_buf(),
                target: "/workspace".into(),
                mode: "direct".into(),
            },
            Mapping {
                source: state_dir.join("home"),
                target: "/home/agent".into(),
                mode: "private_write".into(),
            },
        ];
        let mut mount_specs = mappings
            .iter()
            .map(|mapping| format!("{}:{}", mapping.source.display(), mapping.target))
            .collect::<Vec<_>>();
        for volume in volumes {
            let source_path = fs::canonicalize(&volume.source).with_context(|| {
                format!("invalid environment volume {}", volume.source.display())
            })?;
            validate_target(&volume.target)?;
            let mode = if volume.read_only {
                "read_only"
            } else {
                "direct"
            };
            mount_specs.push(format!(
                "{}:{}{}",
                source_path.display(),
                volume.target,
                if volume.read_only { ":ro" } else { "" }
            ));
            mappings.push(Mapping {
                source: source_path,
                target: volume.target.clone(),
                mode: mode.into(),
            });
        }
        for path in &tool_paths {
            mappings.push(Mapping {
                source: path.clone(),
                target: path.to_string_lossy().into_owned(),
                mode: "read_only".into(),
            });
        }
        let manifest = EnvironmentManifest {
            version: 1,
            source,
            base_rootfs: rootfs.clone(),
            workspace: "/workspace".into(),
            home: "/home/agent".into(),
            cache: "/home/agent/.cache".into(),
            mappings,
        };
        fs::write(
            state_dir.join("environment.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;

        let mut env = vec![
            "HOME=/home/agent".into(),
            "XDG_CACHE_HOME=/home/agent/.cache".into(),
            "LANG=C.UTF-8".into(),
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
        ];
        env.extend(requested_env.iter().cloned());
        // A named profile owns tool discovery and cache locations. Generic task
        // environment values may customize LANG/TERM, but cannot shadow those
        // profile invariants.
        env.extend(profile_env);
        Ok(PreparedEnvironment {
            rootfs_work_dir: state_dir.join("rootfs"),
            state_dir: state_dir.clone(),
            rootfs,
            volumes: mount_specs,
            env,
            filesystem_paths: vec![
                (PathBuf::from("/workspace"), FsAccess::ReadWriteExecute),
                (PathBuf::from("/home/agent"), FsAccess::ReadWriteExecute),
            ]
            .into_iter()
            .chain(
                tool_paths
                    .into_iter()
                    .map(|path| (path, FsAccess::ReadExecute)),
            )
            .collect(),
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&state_dir);
    }
    result
}

fn resolve_source(
    request: &EnvironmentRequest,
) -> Result<(String, PathBuf, Vec<String>, Vec<PathBuf>)> {
    match request {
        EnvironmentRequest::Host => Ok(("host".into(), PathBuf::from("/"), Vec::new(), Vec::new())),
        EnvironmentRequest::Rootfs { path } => {
            let path = fs::canonicalize(path).context("invalid environment rootfs")?;
            if !path.is_dir() {
                bail!("environment rootfs must be a directory");
            }
            Ok(("rootfs".into(), path, Vec::new(), Vec::new()))
        }
        EnvironmentRequest::Profile { name } => {
            let (env, paths) = match name.as_str() {
                "host-basic" => (Vec::new(), Vec::new()),
                "rust" => {
                    let (bin, install) = discover_tool("rustc", 2)?;
                    (
                        vec![
                            format!("PATH={}:/usr/bin:/bin", bin.display()),
                            "CARGO_HOME=/home/agent/.cache/cargo".into(),
                        ],
                        vec![bin, install],
                    )
                }
                "node" => {
                    let (bin, install) = discover_tool("node", 1)?;
                    (
                        vec![
                            format!("PATH={}:/usr/bin:/bin", bin.display()),
                            "npm_config_cache=/home/agent/.cache/npm".into(),
                        ],
                        vec![bin, install, PathBuf::from("/etc/ssl")],
                    )
                }
                "python" => (
                    vec!["PIP_CACHE_DIR=/home/agent/.cache/pip".into()],
                    Vec::new(),
                ),
                _ => bail!("unknown environment profile: {name}"),
            };
            Ok((format!("profile:{name}"), PathBuf::from("/"), env, paths))
        }
    }
}

fn discover_tool(name: &str, install_parents: usize) -> Result<(PathBuf, PathBuf)> {
    let output = std::process::Command::new("sh")
        .args(["-c", "command -v -- \"$1\"", "tinybox-profile", name])
        .output()?;
    let command_path = if output.status.success() {
        PathBuf::from(String::from_utf8(output.stdout)?.trim())
    } else {
        discover_login_tool(name).context(format!("profile requires host tool: {name}"))?
    };
    let resolved = fs::canonicalize(&command_path)?;
    // Use the resolved binary directory rather than a rustup/nvm shim path.
    // This avoids importing user configuration or credentials merely to pick
    // a toolchain version.
    let bin = resolved
        .parent()
        .context("tool has no bin directory")?
        .to_path_buf();
    let mut install = resolved
        .parent()
        .context("tool has no install directory")?
        .to_path_buf();
    for _ in 0..install_parents {
        install = install
            .parent()
            .context("tool install path is too shallow")?
            .to_path_buf();
    }
    Ok((bin, install))
}

fn discover_login_tool(name: &str) -> Option<PathBuf> {
    let user = std::env::var("SUDO_USER").ok()?;
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    let home = passwd.lines().find_map(|line| {
        let fields = line.split(':').collect::<Vec<_>>();
        (fields.len() >= 6 && fields[0] == user).then(|| PathBuf::from(fields[5]))
    })?;
    if matches!(name, "rustc" | "cargo") {
        let toolchains = home.join(".rustup/toolchains");
        let mut candidates = fs::read_dir(toolchains)
            .ok()?
            .filter_map(|entry| entry.ok().map(|entry| entry.path().join("bin").join(name)))
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        candidates.sort_by_key(|path| (!path.to_string_lossy().contains("stable"), path.clone()));
        return candidates.into_iter().next();
    }
    if matches!(name, "node" | "npm") {
        let versions = home.join(".nvm/versions/node");
        let mut candidates = fs::read_dir(versions)
            .ok()?
            .filter_map(|entry| entry.ok().map(|entry| entry.path().join("bin").join(name)))
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        candidates.sort();
        return candidates.pop();
    }
    None
}

fn validate_target(target: &str) -> Result<()> {
    let path = Path::new(target);
    if !path.is_absolute()
        || path == Path::new("/")
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        bail!("volume target must be a normalized absolute non-root path");
    }
    if target == "/workspace" || target.starts_with("/home/agent") {
        bail!("volume target overlaps a managed environment path");
    }
    Ok(())
}

pub fn remove_state_dir(path: &Path) -> Result<()> {
    let root = Path::new("/var/lib/tinybox/tasks");
    if path.parent() != Some(root) {
        bail!("refusing to remove unrecognized task state path");
    }
    if path.exists() {
        fs::remove_dir_all(path).context("failed to remove task state directory")?;
    }
    Ok(())
}

pub fn cleanup_orphaned_state_dirs() -> Result<()> {
    let root = Path::new("/var/lib/tinybox/tasks");
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let Some(id) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if id.starts_with("task-")
            && !Path::new("/sys/fs/cgroup")
                .join(format!("tinybox-{id}"))
                .exists()
        {
            remove_state_dir(&path)?;
        }
    }
    Ok(())
}
