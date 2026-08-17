use anyhow::{Context, Result};
use nix::sys::socket::{recvmsg, ControlMessageOwned, MsgFlags};
use std::io::{IoSliceMut, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::UnixDatagram;

pub const PROXY_ADDRESS: &str = "127.0.0.1:18080";

pub fn serve(channel: UnixDatagram, ready_fd: i32) -> Result<()> {
    bring_loopback_up()?;
    let listener = TcpListener::bind(PROXY_ADDRESS).context("failed to bind sandbox proxy")?;
    let ready = [1u8];
    let written = unsafe { libc::write(ready_fd, ready.as_ptr().cast(), ready.len()) };
    unsafe {
        libc::close(ready_fd);
    }
    if written != 1 {
        anyhow::bail!("failed to signal sandbox proxy readiness");
    }
    crate::seccomp::drop_capabilities(false)?;
    for client in listener.incoming() {
        let mut client = client.context("proxy accept failed")?;
        if let Err(error) = handle_client(&mut client, &channel) {
            let _ = write!(client, "HTTP/1.1 403 Forbidden\r\n\r\n{error}\n");
        }
    }
    Ok(())
}

fn handle_client(client: &mut TcpStream, channel: &UnixDatagram) -> Result<()> {
    let header = read_header(client)?;
    let first_line = header.lines().next().context("empty proxy request")?;
    let mut fields = first_line.split_whitespace();
    if fields.next() != Some("CONNECT") {
        anyhow::bail!("only HTTP CONNECT is supported");
    }
    let destination = fields.next().context("CONNECT destination is missing")?;
    validate_destination(destination)?;
    channel
        .send(destination.as_bytes())
        .context("failed to request broker connection")?;
    let remote = receive_fd(channel)?;
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    relay(client, remote)
}

fn read_header(client: &mut TcpStream) -> Result<String> {
    let mut bytes = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    while bytes.len() < 8192 {
        client.read_exact(&mut byte)?;
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return String::from_utf8(bytes).context("proxy header is not UTF-8");
        }
    }
    anyhow::bail!("proxy header exceeds 8192 bytes")
}

fn validate_destination(destination: &str) -> Result<()> {
    let (host, port) = destination
        .rsplit_once(':')
        .context("CONNECT destination must be HOST:PORT")?;
    if host.is_empty()
        || host != host.to_ascii_lowercase()
        || host.contains(['/', '@', '[', ']'])
        || port.parse::<u16>().is_err()
    {
        anyhow::bail!("invalid CONNECT destination")
    }
    Ok(())
}

fn receive_fd(channel: &UnixDatagram) -> Result<TcpStream> {
    let mut response = [0u8; 512];
    let mut iov = [IoSliceMut::new(&mut response)];
    let mut cmsgspace = nix::cmsg_space!([RawFd; 1]);
    let (bytes, received_fd) = {
        let message = recvmsg::<()>(
            channel.as_raw_fd(),
            &mut iov,
            Some(&mut cmsgspace),
            MsgFlags::empty(),
        )?;
        let mut received_fd = None;
        for cmsg in message.cmsgs()? {
            if let ControlMessageOwned::ScmRights(fds) = cmsg {
                if let Some(fd) = fds.first() {
                    received_fd = Some(*fd);
                }
            }
        }
        (message.bytes, received_fd)
    };
    if let Some(fd) = received_fd {
        // SAFETY: SCM_RIGHTS returned a new owned descriptor.
        return Ok(unsafe { TcpStream::from_raw_fd(fd) });
    }
    let text = String::from_utf8_lossy(&response[..bytes]);
    anyhow::bail!("broker denied connection: {text}")
}

fn relay(client: &TcpStream, remote: TcpStream) -> Result<()> {
    let mut open = [true, true];
    let mut buffer = [0u8; 16 * 1024];
    while open[0] || open[1] {
        let mut poll_fds = [
            libc::pollfd {
                fd: client.as_raw_fd(),
                events: if open[0] { libc::POLLIN } else { 0 },
                revents: 0,
            },
            libc::pollfd {
                fd: remote.as_raw_fd(),
                events: if open[1] { libc::POLLIN } else { 0 },
                revents: 0,
            },
        ];
        let result = unsafe { libc::poll(poll_fds.as_mut_ptr(), poll_fds.len() as _, -1) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error).context("proxy relay poll failed");
        }
        for index in 0..2 {
            let events = poll_fds[index].revents;
            if events & (libc::POLLERR | libc::POLLNVAL) != 0 {
                anyhow::bail!("proxy relay socket failed");
            }
            if !open[index] || events & (libc::POLLIN | libc::POLLHUP) == 0 {
                continue;
            }
            let (source, destination) = if index == 0 {
                (client, &remote)
            } else {
                (&remote, client)
            };
            let count = (&*source).read(&mut buffer)?;
            if count == 0 {
                open[index] = false;
                let _ = destination.shutdown(Shutdown::Write);
            } else {
                (&*destination).write_all(&buffer[..count])?;
            }
        }
    }
    Ok(())
}

fn bring_loopback_up() -> Result<()> {
    #[repr(C)]
    struct IfReq {
        name: [libc::c_char; libc::IFNAMSIZ],
        data: [u8; 24],
    }
    let socket = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if socket < 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to open loopback ioctl socket");
    }
    let mut request = IfReq {
        name: [0; libc::IFNAMSIZ],
        data: [0; 24],
    };
    for (target, source) in request.name.iter_mut().zip(b"lo\0") {
        *target = *source as libc::c_char;
    }
    let get_result = unsafe { libc::ioctl(socket, libc::SIOCGIFFLAGS, &mut request) };
    if get_result == 0 {
        let flags = unsafe { &mut *(request.data.as_mut_ptr().cast::<libc::c_short>()) };
        *flags |= libc::IFF_UP as libc::c_short;
    }
    let set_result = if get_result == 0 {
        unsafe { libc::ioctl(socket, libc::SIOCSIFFLAGS, &request) }
    } else {
        -1
    };
    unsafe {
        libc::close(socket);
    }
    if set_result != 0 {
        return Err(std::io::Error::last_os_error()).context("failed to bring loopback up");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_connect_destinations() {
        assert!(validate_destination("example.com:443").is_ok());
        assert!(validate_destination("EXAMPLE.com:443").is_err());
        assert!(validate_destination("user@example.com:443").is_err());
    }
}
