use crate::audit::AuditSink;
use crate::policy::NetworkRule;
use anyhow::{Context, Result};
use nix::sys::socket::{sendmsg, ControlMessage, MsgFlags};
use std::io::{ErrorKind, IoSlice};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::os::fd::{AsRawFd, IntoRawFd};
use std::os::unix::net::UnixDatagram;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub fn serve(
    channel: UnixDatagram,
    rules: Arc<RwLock<Vec<NetworkRule>>>,
    stop: Arc<AtomicBool>,
    audit: Option<AuditSink>,
) -> Result<()> {
    channel
        .set_read_timeout(Some(Duration::from_millis(100)))
        .context("failed to configure broker channel")?;
    let mut request = [0u8; 1024];
    loop {
        let size = match channel.recv(&mut request) {
            Ok(size) => size,
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                continue;
            }
            Err(error) => return Err(error).context("broker receive failed"),
        };
        if size == 0 {
            return Ok(());
        }
        let request = std::str::from_utf8(&request[..size]).context("invalid broker request")?;
        let current_rules = rules.read().unwrap().clone();
        let rule_id = current_rules
            .iter()
            .position(|rule| request.trim() == format!("{}:{}", rule.host, rule.port))
            .map(|index| format!("network:{index}"));
        let response = match connect_allowed(request, &current_rules) {
            Ok(stream) => {
                if let Some(audit) = &audit {
                    audit.record(
                        "broker",
                        "allow",
                        "network.connect",
                        request.trim(),
                        rule_id,
                        "destination matched policy and connected",
                    );
                }
                send_fd(&channel, stream.into_raw_fd())?;
                continue;
            }
            Err(error) => {
                if let Some(audit) = &audit {
                    audit.record(
                        "broker",
                        "deny",
                        "network.connect",
                        request.trim(),
                        rule_id,
                        error.to_string(),
                    );
                }
                format!("DENY:{error}")
            }
        };
        channel
            .send(response.as_bytes())
            .context("failed to return broker denial")?;
    }
}

fn connect_allowed(request: &str, rules: &[NetworkRule]) -> Result<TcpStream> {
    let (host, port) = request
        .trim()
        .rsplit_once(':')
        .context("request must be HOST:PORT")?;
    let port: u16 = port.parse().context("invalid destination port")?;
    if !rules
        .iter()
        .any(|rule| rule.host == host && rule.port == port)
    {
        anyhow::bail!("destination is not allowlisted");
    }
    let addresses: Vec<_> = (host, port)
        .to_socket_addrs()
        .context("DNS resolution failed")?
        .collect();
    if addresses.is_empty() {
        anyhow::bail!("DNS returned no addresses");
    }
    let localhost_fixture = host == "localhost";
    for address in addresses {
        if forbidden_ip(address.ip()) && !localhost_fixture {
            continue;
        }
        if let Ok(stream) = TcpStream::connect_timeout(&address, std::time::Duration::from_secs(5))
        {
            return Ok(stream);
        }
    }
    anyhow::bail!("no approved destination address was reachable")
}

fn forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

fn send_fd(channel: &UnixDatagram, fd: i32) -> Result<()> {
    let payload = [IoSlice::new(b"OK")];
    let fds = [fd];
    let result = sendmsg::<()>(
        channel.as_raw_fd(),
        &payload,
        &[ControlMessage::ScmRights(&fds)],
        MsgFlags::empty(),
        None,
    )
    .context("failed to pass connected socket");
    unsafe {
        libc::close(fd);
    }
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_addresses_except_explicit_local_fixture() {
        assert!(forbidden_ip("127.0.0.1".parse().unwrap()));
        assert!(forbidden_ip("169.254.169.254".parse().unwrap()));
        assert!(!forbidden_ip("1.1.1.1".parse().unwrap()));
    }
}
