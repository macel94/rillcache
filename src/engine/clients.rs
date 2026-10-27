//! Bounded process-local client registry for CLIENT LIST/KILL and INFO clients.

use super::Store;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::Ordering;
use std::time::Instant;

#[derive(Debug)]
pub struct ClientRecord {
    pub(super) id: u64,
    pub(super) addr: String,
    pub(super) laddr: String,
    pub(super) name: Option<Vec<u8>>,
    pub(super) library_name: Option<Vec<u8>>,
    pub(super) library_version: Option<Vec<u8>>,
    pub(super) resp3: bool,
    pub(super) user: Option<Vec<u8>>,
    pub(super) created: Instant,
    pub(super) blocked: std::sync::atomic::AtomicBool,
    pub(super) control: Option<TcpStream>,
}

#[derive(Clone, Debug)]
pub struct ClientSnapshot {
    pub id: u64,
    pub addr: String,
    pub laddr: String,
    pub name: Option<Vec<u8>>,
    pub library_name: Option<Vec<u8>>,
    pub library_version: Option<Vec<u8>>,
    pub resp3: bool,
    pub user: Vec<u8>,
    pub age_seconds: u64,
    pub age_ms: u64,
    pub blocked: bool,
    pub sub: usize,
    pub psub: usize,
}

const PAUSE_WRITE_ONLY: u64 = 1 << 63;

impl Store {
    /// Pause client command processing until `timeout_ms` milliseconds from the
    /// current clock. A later deadline wins; an ALL pause is not downgraded by a
    /// later WRITE pause. The deadline and mode live in one atomic word so a
    /// command observes one coherent pause decision.
    pub(crate) fn pause_clients(&self, timeout_ms: i64, writes_only: bool) {
        let now = u64::try_from(self.clock.now_ms().max(0)).unwrap_or(0);
        let end = now.saturating_add(u64::try_from(timeout_ms).unwrap_or(0));
        let mut current = self.client_pause.load(Ordering::Relaxed);
        loop {
            let deadline = current & !PAUSE_WRITE_ONLY;
            let active = deadline > now;
            let mode = if writes_only && (!active || current & PAUSE_WRITE_ONLY != 0) {
                PAUSE_WRITE_ONLY
            } else {
                0
            };
            let deadline = if active && deadline > end {
                deadline
            } else {
                end
            };
            let candidate = deadline | mode;
            match self.client_pause.compare_exchange_weak(
                current,
                candidate,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    pub(crate) fn unpause_clients(&self) {
        self.client_pause.store(0, Ordering::Relaxed);
    }

    /// Returns `Some(writes_only)` while a pause is active.
    pub(crate) fn client_pause_mode(&self) -> Option<bool> {
        let now = u64::try_from(self.clock.now_ms().max(0)).unwrap_or(0);
        let current = self.client_pause.load(Ordering::Relaxed);
        (current & !PAUSE_WRITE_ONLY > now).then_some(current & PAUSE_WRITE_ONLY != 0)
    }

    pub(crate) fn client_address(&self, id: u64) -> Option<String> {
        self.clients
            .read()
            .ok()?
            .get(&id)
            .map(|client| client.addr.clone())
    }

    pub fn register_client(
        &self,
        id: u64,
        addr: SocketAddr,
        laddr: SocketAddr,
        control: TcpStream,
    ) {
        if let Ok(mut clients) = self.clients.write() {
            clients.insert(
                id,
                ClientRecord {
                    id,
                    addr: addr.to_string(),
                    laddr: laddr.to_string(),
                    name: None,
                    library_name: None,
                    library_version: None,
                    resp3: false,
                    user: None,
                    created: Instant::now(),
                    blocked: std::sync::atomic::AtomicBool::new(false),
                    control: Some(control),
                },
            );
        }
    }

    pub fn unregister_client(&self, id: u64) {
        self.remove_monitor(id);
        if let Ok(mut clients) = self.clients.write() {
            clients.remove(&id);
        }
    }

    pub fn update_client_metadata(
        &self,
        id: u64,
        name: Option<&[u8]>,
        library_name: Option<&[u8]>,
        library_version: Option<&[u8]>,
        resp3: bool,
        user: Option<&[u8]>,
    ) {
        if let Ok(mut clients) = self.clients.write()
            && let Some(record) = clients.get_mut(&id)
        {
            record.name = name.map(<[u8]>::to_vec);
            record.library_name = library_name.map(<[u8]>::to_vec);
            record.library_version = library_version.map(<[u8]>::to_vec);
            record.resp3 = resp3;
            record.user = user.map(<[u8]>::to_vec);
        }
    }

    pub fn connected_clients(&self) -> usize {
        self.clients.read().map_or(0, |clients| clients.len())
    }

    pub fn client_snapshots(&self, type_filter: Option<bool>) -> Vec<ClientSnapshot> {
        let Ok(clients) = self.clients.read() else {
            return Vec::new();
        };
        let mut snapshots = Vec::with_capacity(clients.len());
        for record in clients.values() {
            let (sub, psub) = self.pubsub_counts(record.id);
            let pubsub = sub + psub > 0;
            if type_filter.is_some_and(|wanted| wanted != pubsub) {
                continue;
            }
            snapshots.push(ClientSnapshot {
                id: record.id,
                addr: record.addr.clone(),
                laddr: record.laddr.clone(),
                name: record.name.clone(),
                library_name: record.library_name.clone(),
                library_version: record.library_version.clone(),
                resp3: record.resp3,
                user: record.user.clone().unwrap_or_else(|| b"default".to_vec()),
                age_seconds: record.created.elapsed().as_secs(),
                age_ms: u64::try_from(record.created.elapsed().as_millis()).unwrap_or(u64::MAX),
                blocked: record.blocked.load(std::sync::atomic::Ordering::Relaxed),
                sub,
                psub,
            });
        }
        snapshots.sort_unstable_by_key(|snapshot| snapshot.id);
        snapshots
    }

    pub fn set_client_blocked(&self, id: u64, blocked: bool) {
        if let Ok(clients) = self.clients.read()
            && let Some(record) = clients.get(&id)
        {
            record
                .blocked
                .store(blocked, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn blocked_clients(&self) -> usize {
        self.clients.read().map_or(0, |clients| {
            clients
                .values()
                .filter(|record| record.blocked.load(std::sync::atomic::Ordering::Relaxed))
                .count()
        })
    }

    pub fn client_snapshot(&self, id: u64) -> Option<ClientSnapshot> {
        self.client_snapshots(None)
            .into_iter()
            .find(|snapshot| snapshot.id == id)
    }

    /// Close every listed client except `skip`. Controls are cloned under the
    /// registry lock and shut down after it is released.
    pub fn client_kill_ids(&self, ids: &[u64], skip: u64) -> usize {
        let controls = {
            let Ok(mut clients) = self.clients.write() else {
                return 0;
            };
            let mut controls = Vec::new();
            for id in ids {
                if *id == skip {
                    continue;
                }
                if let Some(record) = clients.remove(id)
                    && let Some(control) = record.control
                    && let Ok(clone) = control.try_clone()
                {
                    controls.push(clone);
                }
            }
            controls
        };
        for control in &controls {
            let _ = control.shutdown(std::net::Shutdown::Both);
        }
        controls.len()
    }

    pub(super) fn pubsub_counts(&self, id: u64) -> (usize, usize) {
        self.pubsub
            .read()
            .map_or((0, 0), |pubsub| pubsub.counts_for(id))
    }
}

#[cfg(test)]
mod pause_tests {
    use super::*;

    #[test]
    fn pause_deadline_mode_and_unpause_are_coherent() {
        let store = Store::new(1024);
        assert_eq!(store.client_pause_mode(), None);
        store.pause_clients(30, true);
        assert_eq!(store.client_pause_mode(), Some(true));
        store.pause_clients(1000, false);
        assert_eq!(store.client_pause_mode(), Some(false));
        store.pause_clients(1000, true);
        assert_eq!(store.client_pause_mode(), Some(false));
        store.unpause_clients();
        assert_eq!(store.client_pause_mode(), None);
        store.pause_clients(0, false);
        assert_eq!(store.client_pause_mode(), None);
        std::thread::sleep(std::time::Duration::from_millis(20));
        store.pause_clients(10, true);
        assert_eq!(store.client_pause_mode(), Some(true));
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert_eq!(store.client_pause_mode(), None);
    }
}
