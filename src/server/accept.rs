//! Best-effort bounded kernel accept waits; no new readiness dependency/thread.
//! Linux applies SO_RCVTIMEO to accept as well as reads. Other platforms, or
//! failed setup, retain the existing nonblocking accept/backoff loop.

use std::net::TcpListener;

#[cfg(target_os = "linux")]
use std::{io, net::TcpStream, os::fd::OwnedFd, time::Duration};

#[cfg(target_os = "linux")]
const WAIT: Duration = Duration::from_millis(10);

#[cfg(target_os = "linux")]
fn configure_timeout(
    set_timeout: impl FnOnce() -> io::Result<()>,
    set_blocking: impl FnOnce() -> io::Result<()>,
) -> bool {
    // Never make accept blocking unless its kernel deadline was installed.
    set_timeout().is_ok() && set_blocking().is_ok()
}

#[cfg(target_os = "linux")]
pub(super) fn enable_timeout(listener: &TcpListener) -> bool {
    let Ok(clone) = listener.try_clone() else {
        return false;
    };
    // A safe, owned duplicate is used ONLY for the shared socket option; no
    // stream I/O or shutdown is performed through it. Drop closes the duplicate,
    // not the original listening descriptor. Accepted client timeouts are still
    // explicitly set from Config before serving any client I/O.
    let control = TcpStream::from(OwnedFd::from(clone));
    configure_timeout(
        || control.set_read_timeout(Some(WAIT)),
        || listener.set_nonblocking(false),
    )
}

#[cfg(not(target_os = "linux"))]
pub(super) fn enable_timeout(_listener: &TcpListener) -> bool {
    false
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn failed_timeout_setup_never_enables_blocking_accept() {
        let blocking_called = std::cell::Cell::new(false);
        assert!(!configure_timeout(
            || Err(io::Error::other("unsupported timeout")),
            || {
                blocking_called.set(true);
                Ok(())
            },
        ));
        assert!(!blocking_called.get());
    }

    #[test]
    fn timeout_is_installed_before_blocking_mode_even_when_that_step_fails() {
        let order = std::cell::RefCell::new(Vec::new());
        assert!(!configure_timeout(
            || {
                order.borrow_mut().push("timeout");
                Ok(())
            },
            || {
                order.borrow_mut().push("blocking");
                Err(io::Error::other("mode change failed"))
            },
        ));
        assert_eq!(*order.borrow(), ["timeout", "blocking"]);
    }

    #[test]
    fn an_empty_accept_has_a_positive_timeout_and_the_listener_remains_usable() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        assert!(enable_timeout(&listener));
        let options = TcpStream::from(OwnedFd::from(listener.try_clone().unwrap()));
        let timeout = options.read_timeout().unwrap().expect("bounded accept");
        // Linux may round a socket timeout to the timer tick. Do not impose a
        // real-time wall-clock upper bound on a descheduled test thread.
        assert!(timeout >= WAIT && timeout <= Duration::from_millis(100));
        let started = Instant::now();
        let error = listener.accept().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(started.elapsed() >= WAIT / 2);

        let peer =
            TcpStream::connect_timeout(&listener.local_addr().unwrap(), Duration::from_secs(1))
                .unwrap();
        let (accepted, _) = listener.accept().unwrap();
        accepted
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        assert_eq!(
            accepted.read_timeout().unwrap(),
            Some(Duration::from_secs(30))
        );
        assert_eq!(options.read_timeout().unwrap(), Some(timeout));
        drop(peer);
    }
}
