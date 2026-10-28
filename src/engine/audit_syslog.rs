//! Optional best-effort off-host audit shipping.
//!
//! This is a deliberately small UDP sink. It does not retry, authenticate,
//! encrypt or guarantee delivery; the local append-only file remains the durable
//! evidence path.

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

pub(super) struct SyslogSink {
    socket: UdpSocket,
    destination: SocketAddr,
}

impl SyslogSink {
    pub(super) fn open(destination: SocketAddr) -> io::Result<Self> {
        if destination.port() == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "audit syslog destination needs a nonzero port",
            ));
        }
        let bind = if destination.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = UdpSocket::bind(bind)?;
        socket.set_write_timeout(Some(Duration::from_millis(100)))?;
        Ok(Self {
            socket,
            destination,
        })
    }

    pub(super) fn send(&self, line: &str) -> io::Result<()> {
        self.socket.send_to(line.as_bytes(), self.destination)?;
        Ok(())
    }
}
