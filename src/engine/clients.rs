//! Bounded process-local client registry for CLIENT LIST/KILL and INFO clients.

use super::Store;
use std::net::{SocketAddr, TcpStream};
use std::time::Instant;

#[derive(Debug)]
pub struct ClientRecord {
    pub(super) id: u64,
    pub(super) addr: String,
    pub(super) name: Option<Vec<u8>>,
    pub(super) library_name: Option<Vec<u8>>,
    pub(super) library_version: Option<Vec<u8>>,
    pub(super) resp3: bool,
    pub(super) user: Option<Vec<u8>>,
    pub(super) created: Instant,
    pub(super) control: Option<TcpStream>,
}

#[derive(Clone, Debug)]
pub struct ClientSnapshot {
    pub id: u64,
    pub addr: String,
    pub name: Option<Vec<u8>>,
    pub library_name: Option<Vec<u8>>,
    pub library_version: Option<Vec<u8>>,
    pub resp3: bool,
    pub user: Vec<u8>,
    pub age_seconds: u64,
    pub sub: usize,
    pub psub: usize,
}

impl Store {
    pub(crate) fn client_address(&self, id: u64) -> Option<String> {
        self.clients
            .read()
            .ok()?
            .get(&id)
            .map(|client| client.addr.clone())
    }

    pub fn register_client(&self, id: u64, addr: SocketAddr, control: TcpStream) {
        if let Ok(mut clients) = self.clients.write() {
            clients.insert(
                id,
                ClientRecord {
                    id,
                    addr: addr.to_string(),
                    name: None,
                    library_name: None,
                    library_version: None,
                    resp3: false,
                    user: None,
                    created: Instant::now(),
                    control: Some(control),
                },
            );
        }
    }

    pub fn unregister_client(&self, id: u64) {
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
                name: record.name.clone(),
                library_name: record.library_name.clone(),
                library_version: record.library_version.clone(),
                resp3: record.resp3,
                user: record.user.clone().unwrap_or_else(|| b"default".to_vec()),
                age_seconds: record.created.elapsed().as_secs(),
                sub,
                psub,
            });
        }
        snapshots.sort_unstable_by_key(|snapshot| snapshot.id);
        snapshots
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
