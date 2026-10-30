//! Bounded ACL denial log for SEC-1/SEC-2 operations.

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

impl super::ServerServices {
    pub fn record_acl_denial(&self, reason: &str, context: &str, object: &[u8], username: &[u8]) {
        if let Ok(mut log) = self.acl_log.write() {
            if let Some(entry) = log.iter_mut().find(|entry| {
                entry.reason == reason.as_bytes()
                    && entry.context == context.as_bytes()
                    && entry.object == object
                    && entry.username == username
            }) {
                entry.count = entry.count.saturating_add(1);
            } else {
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
        }
        let key = self.audit_hmac_key.read().expect("audit key lock");
        let line = super::audit::format_line(reason, context, object, username, key.as_deref());
        if let Ok(mut audit) = self.audit_log.lock() {
            let failed = audit
                .as_mut()
                .is_some_and(|log| log.record_line(&line).is_err());
            if failed {
                self.audit_log_write_failures
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if let Ok(sink) = self.audit_syslog.lock() {
            let failed = sink.as_ref().is_some_and(|sink| sink.send(&line).is_err());
            if failed {
                self.audit_syslog_write_failures
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Attach the optional per-line HMAC key. The key is never logged or
    /// included in `Debug` output.
    pub fn set_audit_hmac_key(&self, key: Vec<u8>) {
        *self.audit_hmac_key.write().expect("audit key lock") = Some(key);
    }

    /// Attach a best-effort UDP audit sink. Delivery is not retried or
    /// authenticated; the local file, when configured, remains authoritative.
    pub fn attach_audit_syslog(&self, destination: std::net::SocketAddr) -> std::io::Result<()> {
        *self
            .audit_syslog
            .lock()
            .map_err(|_| std::io::Error::other("audit sink lock"))? =
            Some(super::audit_syslog::SyslogSink::open(destination)?);
        Ok(())
    }

    /// Attach an append-only denial audit. The server calls this once before
    /// accepting clients; `ShardExecutor::new`-based unit tests leave it disabled.
    pub fn attach_audit_log(&self, path: &std::path::Path, max_bytes: u64) -> std::io::Result<()> {
        *self
            .audit_log
            .lock()
            .map_err(|_| std::io::Error::other("audit log lock"))? =
            Some(super::audit::AuditLog::open(path, max_bytes)?);
        Ok(())
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
    use crate::engine::{Access, Auth, ShardExecutor};

    #[test]
    fn denials_aggregate_and_reset_with_a_bounded_log() {
        let store = ShardExecutor::new(1024);
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
        let store = ShardExecutor::with_auth(1024, Auth::from_password(None));
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
