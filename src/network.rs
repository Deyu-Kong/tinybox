use anyhow::{Context, Result};
use std::process::Command;

const BRIDGE_NAME: &str = "tinybox0";
const BRIDGE_IP: &str = "172.20.0.1";
const BRIDGE_SUBNET: &str = "172.20.0.0/16";

pub struct NetworkConfig {
    pub bridge_name: String,
    pub container_ip: String,
    pub veth_host: String,
    pub veth_container: String,
}

impl NetworkConfig {
    pub fn new(container_id: u32) -> Self {
        let octet3 = ((container_id - 1) / 254) + 1;
        let octet4 = ((container_id - 1) % 254) + 2;
        Self {
            bridge_name: BRIDGE_NAME.to_string(),
            container_ip: format!("172.20.{}.{}", octet3, octet4),
            veth_host: format!("veth{}h", container_id),
            veth_container: format!("veth{}c", container_id),
        }
    }
}

pub fn setup_bridge() -> Result<()> {
    let output = Command::new("ip")
        .args(["link", "show", BRIDGE_NAME])
        .output()
        .context("failed to check bridge")?;

    if output.status.success() {
        return Ok(());
    }

    Command::new("ip")
        .args(["link", "add", BRIDGE_NAME, "type", "bridge"])
        .status()
        .context("failed to create bridge")?;

    Command::new("ip")
        .args(["addr", "add", &format!("{}/16", BRIDGE_IP), "dev", BRIDGE_NAME])
        .status()
        .context("failed to assign bridge IP")?;

    Command::new("ip")
        .args(["link", "set", BRIDGE_NAME, "up"])
        .status()
        .context("failed to bring up bridge")?;

    setup_nat_rules()?;

    Ok(())
}

fn setup_nat_rules() -> Result<()> {
    Command::new("iptables")
        .args([
            "-t", "nat", "-A", "POSTROUTING",
            "-s", BRIDGE_SUBNET,
            "!", "-d", BRIDGE_SUBNET,
            "-j", "MASQUERADE",
        ])
        .status()
        .context("failed to setup NAT")?;

    Command::new("iptables")
        .args([
            "-A", "FORWARD",
            "-i", BRIDGE_NAME,
            "-j", "ACCEPT",
        ])
        .status()
        .context("failed to setup forward rule")?;

    Command::new("iptables")
        .args([
            "-A", "FORWARD",
            "-o", BRIDGE_NAME,
            "-m", "state", "--state", "RELATED,ESTABLISHED",
            "-j", "ACCEPT",
        ])
        .status()
        .context("failed to setup return rule")?;

    Ok(())
}

pub fn create_veth_pair(config: &NetworkConfig) -> Result<()> {
    Command::new("ip")
        .args([
            "link", "add",
            &config.veth_host, "type", "veth", "peer", "name", &config.veth_container,
        ])
        .status()
        .context("failed to create veth pair")?;

    Command::new("ip")
        .args(["link", "set", &config.veth_host, "master", &config.bridge_name])
        .status()
        .context("failed to attach veth to bridge")?;

    Command::new("ip")
        .args(["link", "set", &config.veth_host, "up"])
        .status()
        .context("failed to bring up veth host")?;

    Ok(())
}

pub fn move_veth_to_ns(config: &NetworkConfig, pid: u32) -> Result<()> {
    Command::new("ip")
        .args(["link", "set", &config.veth_container, "netns", &pid.to_string()])
        .status()
        .context("failed to move veth to namespace")?;

    Ok(())
}

pub fn configure_container_network(config: &NetworkConfig) -> Result<()> {
    Command::new("ip")
        .args(["link", "set", "lo", "up"])
        .status()
        .context("failed to bring up lo")?;

    Command::new("ip")
        .args(["link", "set", &config.veth_container, "up"])
        .status()
        .context("failed to bring up veth container")?;

    Command::new("ip")
        .args([
            "addr", "add",
            &format!("{}/16", config.container_ip),
            "dev", &config.veth_container,
        ])
        .status()
        .context("failed to assign container IP")?;

    Command::new("ip")
        .args([
            "route", "add", "default", "via", BRIDGE_IP,
        ])
        .status()
        .context("failed to add default route")?;

    Ok(())
}

pub fn cleanup_veth(config: &NetworkConfig) -> Result<()> {
    Command::new("ip")
        .args(["link", "del", &config.veth_host])
        .status()
        .ok();
    Ok(())
}

pub fn setup_port_mapping(host_port: u16, container_ip: &str, container_port: u16) -> Result<()> {
    Command::new("iptables")
        .args([
            "-t", "nat", "-A", "PREROUTING",
            "-p", "tcp", "--dport", &host_port.to_string(),
            "-j", "DNAT", "--to-destination", &format!("{}:{}", container_ip, container_port),
        ])
        .status()
        .context("failed to setup port mapping")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_config_new() {
        let config = NetworkConfig::new(1);
        assert_eq!(config.container_ip, "172.20.1.2");
        assert_eq!(config.veth_host, "veth1h");
        assert_eq!(config.veth_container, "veth1c");

        let config2 = NetworkConfig::new(255);
        assert_eq!(config2.container_ip, "172.20.2.2");
    }
}
