//! Optional bounded I/O/execution lanes using only safe standard-library APIs.
//! Nonblocking round-robin polling is portable, not an epoll/kqueue substitute.

use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{self, Sender};
use std::time::Instant;

const QUANTUM: usize = 64;
const OUTPUT_CHUNK: usize = 64 * 1024;

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

struct Client {
    stream: TcpStream,
    input: Vec<u8>,
    output: Vec<u8>,
    sent: usize,
    closing: bool,
    eof: bool,
    activity: Instant,
    _permit: Permit,
}

impl Drop for Client {
    fn drop(&mut self) {
        // Shut down before close even with unread input, as in blocking mode.
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl Client {
    fn flush(&mut self) -> io::Result<bool> {
        if self.sent == self.output.len() {
            self.output.clear();
            self.sent = 0;
            return Ok(false);
        }
        let end = self.output.len().min(self.sent + OUTPUT_CHUNK);
        match self.stream.write(&self.output[self.sent..end]) {
            Ok(0) => Err(io::ErrorKind::WriteZero.into()),
            Ok(size) => {
                self.sent += size;
                self.activity = Instant::now();
                if self.sent == self.output.len() {
                    self.output.clear();
                    self.sent = 0;
                }
                Ok(true)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    /// None means closed; Some indicates whether this pass made progress.
    fn poll(
        &mut self,
        store: &Store,
        config: &Config,
        chunk: &mut [u8],
    ) -> io::Result<Option<bool>> {
        if self.activity.elapsed() >= config.idle_timeout {
            return Ok(None);
        }
        let mut progress = self.flush()?;
        if !self.output.is_empty() {
            return Ok(Some(progress)); // Backpressure: do not read or execute more.
        }
        if self.closing {
            return Ok(None);
        }
        let mut consumed = 0;
        let mut need_input = false;
        for _ in 0..QUANTUM {
            match parse_request(&self.input[consumed..], config.limits) {
                Ok(Some(request)) => {
                    let action = store.execute(&request.arguments, &mut self.output);
                    consumed += request.consumed;
                    progress = true;
                    if action == Action::Close {
                        self.closing = true;
                        break;
                    }
                    // At most one bounded reply can take us beyond this soft limit.
                    if self.output.len() >= OUTPUT_CHUNK {
                        break;
                    }
                }
                Ok(None) => {
                    need_input = true;
                    break;
                }
                Err(error) => {
                    write!(self.output, "-ERR Protocol error: {error}\r\n")?;
                    self.closing = true;
                    progress = true;
                    break;
                }
            }
        }
        if consumed == self.input.len() {
            self.input.clear();
        } else if consumed > 0 {
            self.input.drain(..consumed);
        }
        progress |= self.flush()?;
        if self.closing || !self.output.is_empty() || !need_input {
            return Ok(Some(progress));
        }
        if self.eof {
            if self.input.is_empty() {
                return Ok(None);
            }
            self.output
                .extend_from_slice(b"-ERR Protocol error: unexpected end of stream\r\n");
            self.closing = true;
            return Ok(Some(true));
        }
        let room = (config.limits.max_frame_bytes - self.input.len()).min(chunk.len());
        if room == 0 {
            self.output
                .extend_from_slice(b"-ERR Protocol error: request frame limit exceeded\r\n");
            self.closing = true;
            return Ok(Some(true));
        }
        match self.stream.read(&mut chunk[..room]) {
            Ok(0) => {
                self.eof = true;
                progress = true;
            }
            Ok(size) => {
                self.input.extend_from_slice(&chunk[..size]);
                self.activity = Instant::now();
                progress = true;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
        Ok(Some(progress))
    }
}

struct Lane {
    sender: Sender<Client>,
    thread: JoinHandle<()>,
}

pub(super) fn serve(
    listener: TcpListener,
    config: Config,
    stopping: Arc<AtomicBool>,
) -> io::Result<()> {
    let count = config.worker_threads.expect("fixed worker configuration");
    let store = Arc::new(Store::new(config.max_data_bytes));
    let admitted = Arc::new(AtomicUsize::new(0));
    let mut lanes: Vec<Lane> = Vec::new();
    let result = (|| {
        for index in 0..count {
            // The shared admission counter bounds the sum of all channel queues,
            // active clients and in-flight transfers by max_clients.
            let (sender, receiver) = mpsc::channel::<Client>();
            let store = Arc::clone(&store);
            let stop = Arc::clone(&stopping);
            let config = config.clone();
            let thread = thread::Builder::new()
                .name(format!("rillcache-io-{index}"))
                .spawn(move || {
                    let mut clients: Vec<Client> = Vec::new();
                    let mut chunk = [0; 16 * 1024];
                    while !stop.load(Ordering::Acquire) {
                        let mut progress = false;
                        while let Ok(client) = receiver.try_recv() {
                            clients.push(client);
                            progress = true;
                        }
                        let mut index = 0;
                        while index < clients.len() {
                            match clients[index].poll(&store, &config, &mut chunk) {
                                Ok(Some(made_progress)) => {
                                    progress |= made_progress;
                                    index += 1;
                                }
                                _ => {
                                    clients.swap_remove(index);
                                }
                            }
                        }
                        if !progress {
                            thread::park_timeout(Duration::from_millis(1));
                        }
                    }
                    // Dropping clients and queued transfers closes every socket and permit.
                })?;
            lanes.push(Lane { sender, thread });
        }
        let mut next = 0;
        while !stopping.load(Ordering::Acquire) {
            if lanes.iter().any(|lane| lane.thread.is_finished()) {
                return Err(io::Error::other("I/O worker exited unexpectedly"));
            }
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            stream.set_nodelay(true)?;
            if admitted.load(Ordering::Acquire) >= config.max_clients {
                stream.set_write_timeout(Some(config.idle_timeout))?;
                let _ = stream.write_all(b"-ERR max number of clients reached\r\n");
                let _ = stream.shutdown(Shutdown::Both);
                continue;
            }
            stream.set_nonblocking(true)?;
            admitted.fetch_add(1, Ordering::AcqRel);
            let client = Client {
                stream,
                input: Vec::with_capacity(8192.min(config.limits.max_frame_bytes)),
                output: Vec::with_capacity(8192),
                sent: 0,
                closing: false,
                eof: false,
                activity: Instant::now(),
                _permit: Permit(Arc::clone(&admitted)),
            };
            lanes[next]
                .sender
                .send(client)
                .map_err(|_| io::Error::other("I/O worker unavailable"))?;
            lanes[next].thread.thread().unpark();
            next = (next + 1) % count;
        }
        Ok(())
    })();
    stopping.store(true, Ordering::Release);
    for lane in &lanes {
        lane.thread.thread().unpark();
    }
    let mut result = result;
    for lane in lanes {
        if let Err(error) = join(lane.thread) {
            result = result.and(Err(error));
        }
    }
    result
}
