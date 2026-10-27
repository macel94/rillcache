//! Bounded ACL denial log for SEC-1/SEC-2 operations.

use super::Store;
use std::time::Instant;

const MAX_ACL_LOG: usize = 128;

#[derive(Clone, Debug)]
pub struct AclLogEntry {
    pub count: u64,
    pub reason: Vec<u8>,
    pub context: Vec<u8>,
    pub object: Vec<u8>,
    pub username: Vec<u8>,
    pub created: Instant,
}

impl AclLogEntry {
    pub fn age_seconds(&self) -> u64 {
        self.created.elapsed().as_secs()
    }
}

impl Store {
    pub fn record_acl_denial(&self, reason: &str, context: &str, object: &[u8], username: &[u8]) {
        let Ok(mut log) = self.acl_log.write() else {
            return;
        };
        if let Some(entry) = log.iter_mut().find(|entry| {
            entry.reason == reason.as_bytes()
                && entry.context == context.as_bytes()
                && entry.object == object
                && entry.username == username
        }) {
            entry.count = entry.count.saturating_add(1);
            return;
        }
        log.push_front(AclLogEntry {
            count: 1,
            reason: reason.as_bytes().to_vec(),
            context: context.as_bytes().to_vec(),
            object: object.to_vec(),
            username: username.to_vec(),
            created: Instant::now(),
        });
        log.truncate(MAX_ACL_LOG);
    }

    pub fn acl_log(&self, count: usize) -> Vec<AclLogEntry> {
        self.acl_log.read().map_or_else(
            |_| Vec::new(),
            |log| log.iter().take(count.min(MAX_ACL_LOG)).cloned().collect(),
        )
    }

    pub fn reset_acl_log(&self) -> bool {
        self.acl_log.write().is_ok_and(|mut log| {
            log.clear();
            true
        })
    }

    pub fn acl_dryrun(&self, username: &[u8], arguments: &[&[u8]]) -> Option<super::Access> {
        self.auth
            .read()
            .ok()
            .and_then(|auth| auth.authorize_user(username, arguments))
    }

    /// Cryptographically sourced hex from `/dev/urandom`; never falls back to a
    /// weaker pseudo-random source when the OS source is unavailable.
    pub fn generate_password(&self, bits: usize) -> Option<String> {
        use std::io::Read;
        let bytes = bits.div_ceil(8).min(128);
        let mut random = vec![0u8; bytes];
        std::fs::File::open("/dev/urandom")
            .ok()?
            .read_exact(&mut random)
            .ok()?;
        let chars = bits.div_ceil(4);
        let mut text = String::with_capacity(chars);
        for byte in random {
            use std::fmt::Write as _;
            let _ = write!(text, "{byte:02x}");
        }
        text.truncate(chars);
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::{Access, Auth, Store};

    #[test]
    fn denials_aggregate_and_reset_with_a_bounded_log() {
        let store = Store::new(1024);
        store.record_acl_denial("command", "toplevel", b"get", b"default");
        store.record_acl_denial("command", "toplevel", b"get", b"default");
        store.record_acl_denial("key", "multi", b"secret", b"alice");
        let log = store.acl_log(10);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].count, 1);
        assert_eq!(log[1].count, 2);
        assert_eq!(log[1].object, b"get");
        assert!(store.reset_acl_log());
        assert!(store.acl_log(10).is_empty());
    }

    #[test]
    fn dryrun_is_read_only_and_generatepass_is_bounded_hex() {
        let store = Store::with_auth(1024, Auth::from_password(None));
        assert!(matches!(
            store.acl_dryrun(b"default", &[b"GET", b"k"]),
            Some(Access::Allowed)
        ));
        assert!(store.acl_dryrun(b"missing", &[b"GET", b"k"]).is_none());
        let password = store.generate_password(256).unwrap();
        assert_eq!(password.len(), 64);
        assert!(password.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(store.generate_password(256).unwrap(), password);
    }
}
