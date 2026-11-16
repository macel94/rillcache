//! Read-only container probe using the application binary and safe std only.
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub fn check(
    address: SocketAddr,
    username: Option<&[u8]>,
    password: Option<&[u8]>,
) -> io::Result<()> {
    check_with_timeout(address, username, password, Duration::from_secs(1))
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "health probe deadline exceeded"))
}

fn check_with_timeout(
    address: SocketAddr,
    username: Option<&[u8]>,
    password: Option<&[u8]>,
    timeout: Duration,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut stream = TcpStream::connect_timeout(&address, remaining(deadline)?)?;
    stream.set_nodelay(true)?;
    if let Some(password) = password {
        let mut request = Vec::new();
        if let Some(username) = username {
            rillcache::protocol::encode_request(&[b"AUTH", username, password], &mut request);
        } else {
            rillcache::protocol::encode_request(&[b"AUTH", password], &mut request);
        }
        exchange(&mut stream, &request, b"+OK\r\n", deadline)?;
    }
    exchange(&mut stream, b"*1\r\n$4\r\nPING\r\n", b"+PONG\r\n", deadline)
}

fn exchange(
    stream: &mut TcpStream,
    mut request: &[u8],
    expected: &[u8],
    deadline: Instant,
) -> io::Result<()> {
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
    while received < expected.len() {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        match stream.read(&mut reply[received..expected.len()]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(size) => received += size,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    remaining(deadline)?;
    if &reply[..expected.len()] != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected health probe reply",
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
                assert_eq!(check(address, None, None).is_ok(), reply == b"+PONG\r\n");
            });
        }
    }

    #[test]
    fn authenticates_binary_password_before_ping_and_never_pings_after_rejection() {
        let password = b"probe\0\r\n\xff";
        for (accepted, username) in [
            (true, None),
            (false, None),
            (true, Some(b"probe-user".as_slice())),
            (false, Some(b"probe-user".as_slice())),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut expected = Vec::new();
                    if let Some(username) = username {
                        rillcache::protocol::encode_request(
                            &[b"AUTH", username, password],
                            &mut expected,
                        );
                    } else {
                        rillcache::protocol::encode_request(&[b"AUTH", password], &mut expected);
                    }
                    let mut request = vec![0; expected.len()];
                    stream.read_exact(&mut request).unwrap();
                    assert_eq!(request, expected);
                    if accepted {
                        for byte in b"+OK\r\n" {
                            stream.write_all(&[*byte]).unwrap();
                        }
                        let mut ping = [0; 14];
                        stream.read_exact(&mut ping).unwrap();
                        assert_eq!(&ping, b"*1\r\n$4\r\nPING\r\n");
                        stream.write_all(b"+PONG\r\n").unwrap();
                    } else {
                        stream
                            .write_all(b"-ERR credential-must-not-leak\r\n")
                            .unwrap();
                        let mut next = [0];
                        // The client closes without sending PING or reflecting the error.
                        match stream.read(&mut next) {
                            Ok(0) => {}
                            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
                            other => panic!("probe did not close after AUTH rejection: {other:?}"),
                        }
                    }
                });
                let result = check(address, username, Some(password));
                assert_eq!(result.is_ok(), accepted);
                if let Err(error) = result {
                    assert!(!error.to_string().contains("credential-must-not-leak"));
                }
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
            assert!(check_with_timeout(address, None, None, Duration::from_millis(100)).is_err());
            assert!(started.elapsed() < Duration::from_secs(3));
            release.send(()).unwrap();
        });
    }
}
