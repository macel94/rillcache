//! One joined, bounded-batch transmit worker. It never owns sessions or data.
//! Each job performs one nonblocking write; WouldBlock returns to the reactor's
//! existing writable-interest path rather than holding the worker on a socket.

use super::pool::OUTPUT_CHUNK;
use mio::{Registry, Token, Waker};
use std::io::{self, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Instant;

pub(super) const MAX_BATCH: usize = 256;

pub(super) struct Job<K> {
    pub key: K,
    pub stream: Arc<TcpStream>,
    pub output: Vec<u8>,
    pub sent: usize,
}

pub(super) struct Completed<K> {
    pub job: Job<K>,
    pub result: io::Result<usize>,
    pub at: Instant,
}

impl<K> Job<K> {
    fn write_once(self) -> Completed<K> {
        let end = self
            .output
            .len()
            .min(self.sent.saturating_add(OUTPUT_CHUNK));
        let result = self.output.get(self.sent..end).map_or_else(
            || Err(io::Error::other("invalid transmit offset")),
            |bytes| (&*self.stream).write(bytes),
        );
        Completed {
            job: self,
            result,
            at: Instant::now(),
        }
    }
}

pub(super) struct Worker<K> {
    sender: Option<SyncSender<Vec<Job<K>>>>,
    completed: Receiver<Completed<K>>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl<K: Send + 'static> Worker<K> {
    pub fn new(registry: &Registry, token: Token) -> io::Result<Self> {
        let wake = Arc::new(Waker::new(registry, token)?);
        // At most one queued batch plus one being written. The reactor bounds
        // batch size and admits at most one job per connection. Completion is
        // nonblocking so teardown cannot deadlock behind a full reply channel.
        let (sender, receiver) = mpsc::sync_channel::<Vec<Job<K>>>(1);
        let (done, completed) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("rillcache-transmit".into())
            .spawn(move || {
                while let Ok(batch) = receiver.recv() {
                    for job in batch {
                        if done.send(job.write_once()).is_err() {
                            return Ok(());
                        }
                    }
                    wake.wake()?;
                }
                Ok(())
            })?;
        Ok(Self {
            sender: Some(sender),
            completed,
            thread: Some(thread),
        })
    }

    pub fn submit(&self, jobs: Vec<Job<K>>) -> Result<(), TrySendError<Vec<Job<K>>>> {
        assert!(jobs.len() <= MAX_BATCH, "bounded transmit batch");
        self.sender
            .as_ref()
            .expect("live transmit worker")
            .try_send(jobs)
    }

    pub fn completed(&self) -> Result<Completed<K>, TryRecvError> {
        self.completed.try_recv()
    }
}

impl<K> Worker<K> {
    pub fn shutdown(&mut self) -> io::Result<()> {
        self.sender.take();
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| io::Error::other("transmit worker panicked"))?,
            None => Ok(()),
        }
    }
}

impl<K> Drop for Worker<K> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::{Shutdown, TcpListener};
    use std::time::Duration;

    fn pair() -> (Arc<TcpStream>, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        (Arc::new(stream), peer)
    }

    #[test]
    fn one_write_is_bounded_and_retains_its_owned_buffer_and_offset() {
        let (stream, _peer) = pair();
        let output = vec![b'x'; OUTPUT_CHUNK + 17];
        let done = Job {
            key: 7,
            stream,
            output,
            sent: 3,
        }
        .write_once();
        assert_eq!(done.job.key, 7);
        assert_eq!(done.job.sent, 3);
        assert_eq!(done.job.output.len(), OUTPUT_CHUNK + 17);
        let written = done.result.unwrap();
        assert!(written > 0 && written <= OUTPUT_CHUNK);
    }

    #[test]
    fn full_admission_returns_the_unmodified_job_without_blocking() {
        let (sender, held) = mpsc::sync_channel(1);
        let (_done, completed) = mpsc::channel();
        let worker = Worker {
            sender: Some(sender),
            completed,
            thread: None,
        };
        let (stream, _peer) = pair();
        worker
            .submit(vec![Job {
                key: 1,
                stream: Arc::clone(&stream),
                output: b"first".to_vec(),
                sent: 0,
            }])
            .unwrap_or_else(|_| panic!("empty queue"));
        match worker.submit(vec![Job {
            key: 2,
            stream,
            output: b"second".to_vec(),
            sent: 2,
        }]) {
            Err(TrySendError::Full(jobs)) => {
                assert_eq!(jobs[0].key, 2);
                assert_eq!(jobs[0].output, b"second");
                assert_eq!(jobs[0].sent, 2);
            }
            _ => panic!("full queue must retain caller ownership"),
        }
        drop(held);
    }

    #[test]
    fn a_failed_socket_does_not_prevent_other_jobs_or_joined_shutdown() {
        let poll = mio::Poll::new().unwrap();
        let mut worker = Worker::new(poll.registry(), Token(99)).unwrap();
        let (broken, _peer) = pair();
        broken.shutdown(Shutdown::Write).unwrap();
        let (good, mut peer) = pair();
        worker
            .submit(vec![
                Job {
                    key: 1,
                    stream: broken,
                    output: b"bad".to_vec(),
                    sent: 0,
                },
                Job {
                    key: 2,
                    stream: good,
                    output: b"good".to_vec(),
                    sent: 0,
                },
            ])
            .unwrap_or_else(|_| panic!("empty queue"));
        let (ack, received) = mpsc::channel();
        let join = thread::spawn(move || {
            worker.shutdown().unwrap();
            let first = worker.completed().unwrap();
            let second = worker.completed().unwrap();
            ack.send((
                first.job.key,
                first.result.is_err(),
                second.job.key,
                second.result.unwrap(),
            ))
            .unwrap();
        });
        assert_eq!(
            received.recv_timeout(Duration::from_secs(5)).unwrap(),
            (1, true, 2, 4)
        );
        join.join().unwrap();
        let mut output = [0; 4];
        peer.read_exact(&mut output).unwrap();
        assert_eq!(&output, b"good");
    }
}
