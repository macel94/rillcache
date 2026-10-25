//! Managed thread-per-client I/O, or an opt-in bounded nonblocking worker pool.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::engine::{Action, Store};
use crate::protocol::{Limits, dispatch_request};

mod pool;

/// Fixed workers keep polling briefly after progress, then return to idle parking.
/// This is bounded active polling, not readiness notification or free CPU work.
pub const FIXED_WORKER_ACTIVE_POLL_US: u64 = 50;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub max_clients: usize,
    /// None: one blocking worker per client. Some(1..=64): fixed I/O/execution lanes.
    pub worker_threads: Option<usize>,
    /// Key and value bytes only, not allocator, map, thread or network overhead.
    pub max_data_bytes: usize,
    pub limits: Limits,
    pub idle_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 6379)),
            max_clients: 128,
            worker_threads: None,
            max_data_bytes: 64 * 1024 * 1024,
            limits: Limits::default(),
            idle_timeout: Duration::from_secs(30),
        }
    }
}

impl Config {
    fn validate(&self) -> io::Result<()> {
        if self
            .worker_threads
            .is_some_and(|count| !(1..=64).contains(&count))
            || self.max_clients == 0
            || self.max_data_bytes == 0
            || self.limits.max_arguments == 0
            || self.limits.max_bulk_bytes == 0
            || self.limits.max_frame_bytes < 8
            || self.limits.max_bulk_bytes > self.limits.max_frame_bytes
            || self.idle_timeout.is_zero()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "limits and timeout must be positive; bulk limit must fit within frame limit",
            ));
        }
        Ok(())
    }
}

pub struct Server {
    listener: TcpListener,
    config: Config,
}

impl Server {
    pub fn bind(config: Config) -> io::Result<Self> {
        config.validate()?;
        let listener = TcpListener::bind(config.bind_addr)?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, config })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Run until an OS-level process termination or an unrecoverable server error.
    /// The standard library has no portable signal-handler API.
    pub fn run(self) -> io::Result<()> {
        self.serve(Arc::new(AtomicBool::new(false)))
    }

    /// Run in a managed background thread, useful for tests and local benchmarks.
    pub fn spawn(self) -> io::Result<ServerHandle> {
        let addr = self.local_addr()?;
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopping);
        let thread = thread::Builder::new()
            .name("rillcache-accept".into())
            .spawn(move || self.serve(stop))?;
        Ok(ServerHandle {
            addr,
            stopping,
            thread: Some(thread),
        })
    }

    fn serve(self, stopping: Arc<AtomicBool>) -> io::Result<()> {
        if self.config.worker_threads.is_some() {
            return pool::serve(self.listener, self.config, stopping);
        }
        let store = Arc::new(Store::new(self.config.max_data_bytes));
        let mut workers: Vec<Worker> = Vec::new();
        let mut maintenance = Instant::now();
        let result = (|| {
            while !stopping.load(Ordering::Acquire) {
                if maintenance.elapsed() >= Duration::from_millis(100) {
                    store.purge_expired(128);
                    maintenance = Instant::now();
                }
                let mut index = 0;
                while index < workers.len() {
                    if workers[index].thread.is_finished() {
                        join(workers.swap_remove(index).thread)?;
                    } else {
                        index += 1;
                    }
                }
                let (mut stream, _) = match self.listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                };
                stream.set_nonblocking(false)?;
                stream.set_nodelay(true)?;
                stream.set_read_timeout(Some(self.config.idle_timeout))?;
                stream.set_write_timeout(Some(self.config.idle_timeout))?;
                if workers.len() >= self.config.max_clients {
                    let _ = stream.write_all(b"-ERR max number of clients reached\r\n");
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                }
                let control = stream.try_clone()?;
                let store = Arc::clone(&store);
                let limits = self.config.limits;
                let thread = thread::Builder::new()
                    .name("rillcache-client".into())
                    .stack_size(256 * 1024)
                    .spawn(move || {
                        // Disconnects, idle timeouts and malformed input affect only this client.
                        let _ = serve_client(&mut stream, &store, limits);
                        let _ = stream.shutdown(Shutdown::Both);
                    })?;
                workers.push(Worker { control, thread });
            }
            Ok(())
        })();
        // Wake blocked readers/writers before joining; no detached workers survive shutdown.
        for worker in &workers {
            let _ = worker.control.shutdown(Shutdown::Both);
        }
        let mut result = result;
        for worker in workers {
            if let Err(error) = join(worker.thread) {
                result = result.and(Err(error));
            }
        }
        result
    }
}

struct Worker {
    control: TcpStream,
    thread: JoinHandle<()>,
}

fn join(thread: JoinHandle<()>) -> io::Result<()> {
    thread
        .join()
        .map_err(|_| io::Error::other("client worker panicked"))
}

pub struct ServerHandle {
    addr: SocketAddr,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl ServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn shutdown(mut self) -> io::Result<()> {
        self.stop()
    }

    fn stop(&mut self) -> io::Result<()> {
        self.stopping.store(true, Ordering::Release);
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| io::Error::other("accept worker panicked"))?,
            None => Ok(()),
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn serve_client(stream: &mut TcpStream, store: &Store, limits: Limits) -> io::Result<()> {
    let mut input = Vec::with_capacity(8192.min(limits.max_frame_bytes));
    let mut output = Vec::with_capacity(8192);
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let mut consumed = 0;
        loop {
            let (action, used) = match dispatch_request(&input[consumed..], limits, |args| {
                store.execute(args, &mut output)
            }) {
                Ok(Some(result)) => result,
                Ok(None) => break,
                Err(error) => {
                    write!(output, "-ERR Protocol error: {error}\r\n")?;
                    stream.write_all(&output)?;
                    return Ok(());
                }
            };
            consumed += used;
            if output.len() >= 64 * 1024 || action == Action::Close {
                stream.write_all(&output)?;
                output.clear();
            }
            if action == Action::Close {
                return Ok(());
            }
        }
        if consumed == input.len() {
            input.clear();
        } else if consumed > 0 {
            input.drain(..consumed);
        }
        if !output.is_empty() {
            stream.write_all(&output)?;
            output.clear();
        }
        let room = (limits.max_frame_bytes - input.len()).min(chunk.len());
        if room == 0 {
            stream.write_all(b"-ERR Protocol error: request frame limit exceeded\r\n")?;
            return Ok(());
        }
        match stream.read(&mut chunk[..room]) {
            Ok(0) => {
                if !input.is_empty() {
                    stream.write_all(b"-ERR Protocol error: unexpected end of stream\r\n")?;
                }
                return Ok(());
            }
            Ok(size) => input.extend_from_slice(&chunk[..size]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_configuration_before_binding() {
        for config in [
            Config {
                worker_threads: Some(0),
                ..Config::default()
            },
            Config {
                worker_threads: Some(65),
                ..Config::default()
            },
            Config {
                max_clients: 0,
                ..Config::default()
            },
            Config {
                max_data_bytes: 0,
                ..Config::default()
            },
            Config {
                idle_timeout: Duration::ZERO,
                ..Config::default()
            },
            Config {
                limits: Limits {
                    max_arguments: 0,
                    ..Limits::default()
                },
                ..Config::default()
            },
            Config {
                limits: Limits {
                    max_frame_bytes: 7,
                    ..Limits::default()
                },
                ..Config::default()
            },
        ] {
            assert_eq!(
                Server::bind(config).err().unwrap().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}
