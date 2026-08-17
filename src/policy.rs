use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};

pub const POLICY_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDescriptor {
    pub version: u32,
    #[serde(default)]
    pub filesystem: Vec<FsRule>,
    #[serde(default)]
    pub network: Vec<NetworkRule>,
    pub resources: ResourcePolicy,
    #[serde(default)]
    pub phases: Vec<PhasePolicy>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FsRule {
    pub path: PathBuf,
    pub access: FsAccess,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FsAccess {
    Read,
    ReadWrite,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NetworkRule {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourcePolicy {
    pub memory_bytes: u64,
    pub cpus: f64,
    pub pids: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PhasePolicy {
    pub name: String,
    #[serde(default)]
    pub network: Vec<NetworkRule>,
    pub resources: ResourcePolicy,
    #[serde(default)]
    pub next: Vec<String>,
}

pub struct LoadedPolicy {
    pub descriptor: CapabilityDescriptor,
    pub hash: String,
}

impl CapabilityDescriptor {
    pub fn load(path: &Path) -> Result<LoadedPolicy> {
        let bytes =
            fs::read(path).with_context(|| format!("failed to read policy {}", path.display()))?;
        let descriptor: Self =
            serde_json::from_slice(&bytes).context("invalid capability policy JSON")?;
        descriptor.compile()
    }

    pub fn compile(self) -> Result<LoadedPolicy> {
        self.validate()?;
        let canonical = serde_json::to_vec(&self).context("failed to canonicalize policy")?;
        let hash = format!("sha256:{:x}", Sha256::digest(canonical));
        Ok(LoadedPolicy {
            descriptor: self,
            hash,
        })
    }

    fn validate(&self) -> Result<()> {
        if self.version != POLICY_VERSION {
            anyhow::bail!(
                "unsupported capability policy version {}; expected {}",
                self.version,
                POLICY_VERSION
            );
        }
        if self.resources.memory_bytes == 0 {
            anyhow::bail!("policy resources.memory_bytes must be positive");
        }
        if !self.resources.cpus.is_finite() || self.resources.cpus <= 0.0 {
            anyhow::bail!("policy resources.cpus must be finite and positive");
        }
        if self.resources.pids == 0 {
            anyhow::bail!("policy resources.pids must be positive");
        }
        for rule in &self.filesystem {
            validate_absolute_normalized_path(&rule.path)?;
        }
        for rule in &self.network {
            validate_host(&rule.host)?;
            if rule.port == 0 {
                anyhow::bail!("network port must be positive");
            }
        }
        let mut phase_names = std::collections::HashSet::new();
        for phase in &self.phases {
            if phase.name.is_empty() || !phase_names.insert(&phase.name) {
                anyhow::bail!("phase names must be non-empty and unique");
            }
            validate_resources(&phase.resources)?;
            if phase.resources.memory_bytes > self.resources.memory_bytes
                || phase.resources.cpus > self.resources.cpus
                || phase.resources.pids > self.resources.pids
            {
                anyhow::bail!("phase {} exceeds the resource ceiling", phase.name);
            }
            for rule in &phase.network {
                validate_host(&rule.host)?;
                if !self.network.contains(rule) {
                    anyhow::bail!("phase {} exceeds the network ceiling", phase.name);
                }
            }
        }
        for phase in &self.phases {
            if phase.next.iter().any(|next| !phase_names.contains(next)) {
                anyhow::bail!("phase {} references an unknown next phase", phase.name);
            }
        }
        Ok(())
    }
}

fn validate_resources(resources: &ResourcePolicy) -> Result<()> {
    if resources.memory_bytes == 0
        || !resources.cpus.is_finite()
        || resources.cpus <= 0.0
        || resources.pids == 0
    {
        anyhow::bail!("phase resources must be finite and positive");
    }
    Ok(())
}

fn validate_absolute_normalized_path(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.as_os_str().is_empty() {
        anyhow::bail!("filesystem capability path must be absolute: {path:?}");
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::CurDir | Component::Prefix(_)
        )
    }) {
        anyhow::bail!("filesystem capability path must be normalized: {path:?}");
    }
    Ok(())
}

fn validate_host(host: &str) -> Result<()> {
    if host.is_empty()
        || host != host.to_ascii_lowercase()
        || host.ends_with('.')
        || host.contains(['/', ':', '@'])
        || host.parse::<IpAddr>().is_ok()
    {
        anyhow::bail!("network host must be a normalized lowercase DNS name: {host}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offline_policy() -> CapabilityDescriptor {
        CapabilityDescriptor {
            version: POLICY_VERSION,
            filesystem: Vec::new(),
            network: Vec::new(),
            resources: ResourcePolicy {
                memory_bytes: 512 * 1024 * 1024,
                cpus: 1.0,
                pids: 50,
            },
            phases: Vec::new(),
        }
    }

    #[test]
    fn stable_hash_for_same_policy() {
        let first = offline_policy().compile().unwrap().hash;
        let second = offline_policy().compile().unwrap().hash;
        assert_eq!(first, second);
        assert!(first.starts_with("sha256:"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let json = r#"{"version":1,"filesystem":[],"network":[],"resources":{"memory_bytes":1,"cpus":1.0,"pids":1},"phases":[],"grant_all":true}"#;
        assert!(serde_json::from_str::<CapabilityDescriptor>(json).is_err());
    }

    #[test]
    fn accepts_normalized_network_capabilities() {
        let mut policy = offline_policy();
        policy.network.push(NetworkRule {
            host: "example.com".into(),
            port: 443,
        });
        assert!(policy.compile().is_ok());
    }

    #[test]
    fn rejects_literal_network_addresses_and_zero_ports() {
        let mut policy = offline_policy();
        policy.network.push(NetworkRule {
            host: "127.0.0.1".into(),
            port: 443,
        });
        assert!(policy.compile().is_err());

        let mut policy = offline_policy();
        policy.network.push(NetworkRule {
            host: "example.com".into(),
            port: 0,
        });
        assert!(policy.compile().is_err());
    }

    #[test]
    fn rejects_invalid_resources() {
        let mut policy = offline_policy();
        policy.resources.cpus = f64::NAN;
        assert!(policy.compile().is_err());
    }

    #[test]
    fn validates_phase_graph_and_ceilings() {
        let mut policy = offline_policy();
        policy.network.push(NetworkRule {
            host: "example.com".into(),
            port: 443,
        });
        policy.phases.push(PhasePolicy {
            name: "install".into(),
            network: policy.network.clone(),
            resources: ResourcePolicy {
                memory_bytes: 256 * 1024 * 1024,
                cpus: 0.5,
                pids: 25,
            },
            next: vec!["build".into()],
        });
        policy.phases.push(PhasePolicy {
            name: "build".into(),
            network: Vec::new(),
            resources: ResourcePolicy {
                memory_bytes: 128 * 1024 * 1024,
                cpus: 0.25,
                pids: 10,
            },
            next: Vec::new(),
        });
        assert!(policy.clone().compile().is_ok());
        policy.phases[0].next = vec!["missing".into()];
        assert!(policy.compile().is_err());
    }
}
