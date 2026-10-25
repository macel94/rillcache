//! Read-only container probe using the application binary and safe std only.
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub fn check(address: SocketAddr) -> io::Result<()> {
    check_with_timeout(address, Duration::from_secs(1))
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "PING deadline exceeded"))
}

fn check_with_timeout(address: SocketAddr, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
    stream.set_nodelay(true)?;
    let mut request = b"*1\r\n$4\r\nPING\r\n".as_slice();
    while !request.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(request) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(size) => request = &request[size..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    let mut reply = [0; 7];
    let mut received = 0;
    while received < reply.len() {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        match stream.read(&mut reply[received..]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(size) => received += size,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    remaining(deadline)?;
    if &reply != b"+PONG\r\n" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected RESP PONG",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn validates_ping_and_fragmented_pong_but_rejects_bad_or_truncated_replies() {
        for reply in [
            b"+PONG\r\n".as_slice(),
            b"+PANG\r\n",
            b"-ERR no\r\n",
            b"+PON",
            b"",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut request = [0; 14];
                    stream.read_exact(&mut request).unwrap();
                    assert_eq!(&request, b"*1\r\n$4\r\nPING\r\n");
                    for byte in reply {
                        let _ = stream.write_all(&[*byte]);
                    }
                });
                assert_eq!(check(address).is_ok(), reply == b"+PONG\r\n");
            });
        }
    }

    #[test]
    fn a_silent_peer_cannot_keep_the_probe_alive() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let (_stream, _) = listener.accept().unwrap();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            let started = Instant::now();
            assert!(check_with_timeout(address, Duration::from_millis(100)).is_err());
            assert!(started.elapsed() < Duration::from_secs(3));
            release.send(()).unwrap();
        });
    }
}
