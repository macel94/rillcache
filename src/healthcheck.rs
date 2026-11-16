//! Read-only container probe using the application binary and safe std only.
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub fn check(
    address: SocketAddr,
    username: Option<&[u8]>,
    password: Option<&[u8]>,
    require_aof: bool,
) -> io::Result<()> {
    check_with_timeout(
        address,
        username,
        password,
        require_aof,
        Duration::from_secs(1),
    )
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
    require_aof: bool,
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
    exchange(&mut stream, b"*1\r\n$4\r\nPING\r\n", b"+PONG\r\n", deadline)?;
    if require_aof {
        check_persistence(&mut stream, deadline)?;
    }
    Ok(())
}

fn send_request(stream: &mut TcpStream, mut request: &[u8], deadline: Instant) -> io::Result<()> {
    while !request.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(request) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(size) => request = &request[size..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn read_exact(stream: &mut TcpStream, reply: &mut [u8], deadline: Instant) -> io::Result<()> {
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
    Ok(())
}

fn exchange(
    stream: &mut TcpStream,
    request: &[u8],
    expected: &[u8],
    deadline: Instant,
) -> io::Result<()> {
    send_request(stream, request, deadline)?;
    let mut reply = [0; 7];
    read_exact(stream, &mut reply[..expected.len()], deadline)?;
    if &reply[..expected.len()] != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected health probe reply",
        ));
    }
    Ok(())
}

fn invalid_persistence() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid or unhealthy journal status",
    )
}

fn check_persistence(stream: &mut TcpStream, deadline: Instant) -> io::Result<()> {
    send_request(
        stream,
        b"*2\r\n$4\r\nINFO\r\n$11\r\npersistence\r\n",
        deadline,
    )?;
    // A bounded RESP2 bulk only: never allocate from an unchecked peer length.
    let mut header = Vec::new();
    while header.len() < 32 {
        let mut byte = [0];
        read_exact(stream, &mut byte, deadline)?;
        header.push(byte[0]);
        if header.ends_with(b"\r\n") {
            break;
        }
    }
    let length = header
        .strip_prefix(b"$")
        .and_then(|value| value.strip_suffix(b"\r\n"))
        .filter(|value| !value.is_empty() && value.iter().all(u8::is_ascii_digit))
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|length| *length <= 16 * 1024)
        .ok_or_else(invalid_persistence)?;
    let mut body = vec![0; length + 2];
    read_exact(stream, &mut body, deadline)?;
    if !body.ends_with(b"\r\n") {
        return Err(invalid_persistence());
    }
    validate_persistence(&body[..length])
}

fn validate_persistence(body: &[u8]) -> io::Result<()> {
    let text = std::str::from_utf8(body).map_err(|_| invalid_persistence())?;
    let required = [
        ("loading", "0"),
        ("aof_enabled", "1"),
        ("aof_last_write_status", "ok"),
        ("aof_last_bgrewrite_status", "ok"),
    ];
    let mut seen = [false; 4];
    for line in text.split("\r\n") {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line.split_once(':').ok_or_else(invalid_persistence)?;
        if let Some(index) = required.iter().position(|(field, _)| *field == name) {
            if seen[index] || value != required[index].1 {
                return Err(invalid_persistence());
            }
            seen[index] = true;
        }
    }
    if !seen.iter().all(|present| *present) {
        return Err(invalid_persistence());
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
                assert_eq!(
                    check(address, None, None, false).is_ok(),
                    reply == b"+PONG\r\n"
                );
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
                let result = check(address, username, Some(password), false);
                assert_eq!(result.is_ok(), accepted);
                if let Err(error) = result {
                    assert!(!error.to_string().contains("credential-must-not-leak"));
                }
            });
        }
    }

    const HEALTHY: &str = "# Persistence\r\nloading:0\r\naof_enabled:1\r\naof_last_write_status:ok\r\naof_last_bgrewrite_status:ok\r\n";

    #[test]
    fn persistence_fields_fail_closed_on_missing_duplicate_or_reported_errors() {
        assert!(validate_persistence(HEALTHY.as_bytes()).is_ok());
        assert!(
            validate_persistence(format!("{HEALTHY}aof_rewrite_in_progress:1\r\n").as_bytes())
                .is_ok()
        );
        for (field, value) in [
            ("loading", "0"),
            ("aof_enabled", "1"),
            ("aof_last_write_status", "ok"),
            ("aof_last_bgrewrite_status", "ok"),
        ] {
            let line = format!("{field}:{value}\r\n");
            for body in [
                HEALTHY.replace(&line, ""),
                HEALTHY.replace(&line, &format!("{field}:err\r\n")),
                format!("{HEALTHY}{line}"),
            ] {
                assert!(
                    validate_persistence(body.as_bytes()).is_err(),
                    "field {field}"
                );
            }
        }
        for body in [
            b"".as_slice(),
            b"not-info",
            b"\xff",
            b"loading:0\naof_enabled:1\n",
        ] {
            assert!(validate_persistence(body).is_err());
        }
    }

    #[test]
    fn persistence_probe_bounds_and_validates_fragmented_bulk_replies() {
        let healthy = [
            format!("${}\r\n", HEALTHY.len()).as_bytes(),
            HEALTHY.as_bytes(),
            b"\r\n",
        ]
        .concat();
        for (accepted, reply) in [
            (true, healthy),
            (false, b"$-1\r\n".to_vec()),
            (false, b"$16385\r\n".to_vec()),
            (false, b"$999999999999999999999999999999999999\r\n".to_vec()),
            (false, b"$3\r\nab".to_vec()),
            (false, b"$3\r\nabcxx".to_vec()),
            (false, b"-NOPERM secret-server-text\r\n".to_vec()),
            (false, b"*0\r\n".to_vec()),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut ping = [0; 14];
                    stream.read_exact(&mut ping).unwrap();
                    assert_eq!(&ping, b"*1\r\n$4\r\nPING\r\n");
                    stream.write_all(b"+PONG\r\n").unwrap();
                    let expected = b"*2\r\n$4\r\nINFO\r\n$11\r\npersistence\r\n";
                    let mut request = vec![0; expected.len()];
                    stream.read_exact(&mut request).unwrap();
                    assert_eq!(request, expected);
                    for byte in &reply {
                        if stream.write_all(&[*byte]).is_err() {
                            break;
                        }
                    }
                });
                let result = check(address, None, None, true);
                assert_eq!(result.is_ok(), accepted);
                if let Err(error) = result {
                    assert!(!error.to_string().contains("secret-server-text"));
                }
            });
        }
    }

    #[test]
    fn persistence_read_uses_the_same_bounded_socket_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut ping = [0; 14];
                stream.read_exact(&mut ping).unwrap();
                stream.write_all(b"+PONG\r\n").unwrap();
                let mut request = [0; 32];
                stream.read_exact(&mut request).unwrap();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            let started = Instant::now();
            let result = check_with_timeout(address, None, None, true, Duration::from_millis(100));
            assert!(result.is_err());
            assert!(started.elapsed() < Duration::from_secs(3));
            release.send(()).unwrap();
        });
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
            assert!(
                check_with_timeout(address, None, None, false, Duration::from_millis(100)).is_err()
            );
            assert!(started.elapsed() < Duration::from_secs(3));
            release.send(()).unwrap();
        });
    }
}
