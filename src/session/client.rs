//! HELLO negotiation, CLIENT metadata/list/kill and connection metadata sync.

use std::io::Write;

use crate::engine::{AuthError, Store};
use crate::protocol::encode_bulk;

use super::command::arity;
use super::{MAX_LIBRARY_FIELD_BYTES, MAX_NAME_BYTES, Protocol, Session};

impl Session {
    pub(super) fn hello(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let (requested, options) = match arguments.split_first() {
            None => (self.protocol, &[][..]),
            Some((&version, rest)) => {
                let protocol = match version {
                    b"2" => Protocol::Resp2,
                    b"3" => Protocol::Resp3,
                    _ => {
                        output.extend_from_slice(b"-NOPROTO unsupported protocol version\r\n");
                        return;
                    }
                };
                (protocol, rest)
            }
        };
        if arguments.is_empty() {
            self.hello_reply(requested, output);
            return;
        }

        // Parse and validate every option before changing either protocol,
        // metadata or authentication state.
        let mut name = None;
        let mut credentials = None;
        let mut index = 0;
        while index < options.len() {
            if options[index].eq_ignore_ascii_case(b"AUTH") {
                if credentials.is_some() || index + 2 >= options.len() {
                    output.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                credentials = Some((options[index + 1], options[index + 2]));
                index += 3;
                continue;
            }
            if options[index].eq_ignore_ascii_case(b"SETNAME") {
                if name.is_some()
                    || index + 1 >= options.len()
                    || !valid_metadata(options[index + 1], MAX_NAME_BYTES)
                {
                    output.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                name = Some(options[index + 1]);
                index += 2;
                continue;
            }
            output.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        if let Some((username, password)) = credentials {
            match store.authenticate_user(Some(username), password) {
                Ok(name) => {
                    self.authenticated_user = Some(name);
                    self.failed_auth = 0;
                    self.close_requested = false;
                }
                Err(AuthError::NoPassword) => {
                    output.extend_from_slice(
                        b"-ERR AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?\r\n",
                    );
                    return;
                }
                Err(AuthError::WrongPassword) => {
                    output.extend_from_slice(
                        b"-WRONGPASS invalid username-password pair or user is disabled.\r\n",
                    );
                    self.note_auth_failure(store);
                    return;
                }
                Err(AuthError::Internal) => {
                    output.extend_from_slice(b"-ERR internal authentication error\r\n");
                    return;
                }
            }
        }
        self.protocol = requested;
        if let Some(name) = name {
            self.set_name(name);
        }
        self.hello_reply(requested, output);
    }

    pub(super) fn hello_reply(&self, protocol: Protocol, output: &mut Vec<u8>) {
        // These fields identify this server without claiming a Redis version,
        // modules, replication role, or unsupported capability.
        enum Value<'a> {
            Bulk(&'a [u8]),
            Integer(u64),
        }
        let fields = [
            (b"server".as_slice(), Value::Bulk(b"rillcache")),
            (
                b"version".as_slice(),
                Value::Bulk(env!("CARGO_PKG_VERSION").as_bytes()),
            ),
            (
                b"proto".as_slice(),
                Value::Integer(if protocol == Protocol::Resp3 { 3 } else { 2 }),
            ),
            (b"id".as_slice(), Value::Integer(self.id)),
            (b"mode".as_slice(), Value::Bulk(b"standalone")),
        ];
        if protocol == Protocol::Resp3 {
            output.extend_from_slice(b"%5\r\n");
        } else {
            output.extend_from_slice(b"*10\r\n");
        }
        for (key, value) in fields {
            encode_bulk(Some(key), output);
            match value {
                Value::Bulk(value) => encode_bulk(Some(value), output),
                Value::Integer(value) => {
                    let _ = write!(output, ":{value}\r\n");
                }
            }
        }
    }
    pub(super) fn sync_client_metadata(&self, store: &Store) {
        let user = store.whoami(self.authenticated_user.as_deref());
        store.update_client_metadata(
            self.id,
            self.name.as_deref(),
            self.library_name.as_deref(),
            self.library_version.as_deref(),
            self.protocol == Protocol::Resp3,
            user.as_deref(),
        );
    }
    pub(super) fn client(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let Some((subcommand, rest)) = arguments.split_first() else {
            output.extend_from_slice(b"-ERR wrong number of arguments for 'client' command\r\n");
            return;
        };
        if subcommand.eq_ignore_ascii_case(b"ID") && rest.is_empty() {
            let _ = write!(output, ":{}\r\n", self.id);
        } else if subcommand.eq_ignore_ascii_case(b"SETNAME") && rest.len() == 1 {
            if !valid_metadata(rest[0], MAX_NAME_BYTES) {
                output.extend_from_slice(b"-ERR invalid client name\r\n");
            } else {
                self.set_name(rest[0]);
                output.extend_from_slice(b"+OK\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"GETNAME") && rest.is_empty() {
            if self.protocol == Protocol::Resp3 && self.name.is_none() {
                output.extend_from_slice(b"_\r\n");
            } else {
                encode_bulk(self.name.as_deref(), output);
            }
        } else if subcommand.eq_ignore_ascii_case(b"SETINFO") && rest.len() == 2 {
            let target = if rest[0].eq_ignore_ascii_case(b"LIB-NAME") {
                &mut self.library_name
            } else if rest[0].eq_ignore_ascii_case(b"LIB-VER") {
                &mut self.library_version
            } else {
                output.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            };
            if !valid_metadata(rest[1], MAX_LIBRARY_FIELD_BYTES) {
                output.extend_from_slice(b"-ERR invalid client metadata\r\n");
            } else {
                *target = Some(rest[1].to_vec());
                output.extend_from_slice(b"+OK\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"PAUSE") {
            if rest.is_empty() || rest.len() > 2 {
                arity("client|pause", output);
                return;
            }
            let Some(timeout) = std::str::from_utf8(rest[0])
                .ok()
                .and_then(|text| text.parse::<i64>().ok())
            else {
                output.extend_from_slice(b"-ERR timeout is not an integer or out of range\r\n");
                return;
            };
            if timeout < 0 {
                output.extend_from_slice(b"-ERR timeout is negative\r\n");
                return;
            }
            let writes_only = match rest.get(1) {
                None => false,
                Some(mode) if mode.eq_ignore_ascii_case(b"WRITE") => true,
                Some(mode) if mode.eq_ignore_ascii_case(b"ALL") => false,
                Some(_) => {
                    output.extend_from_slice(b"-ERR CLIENT PAUSE mode must be WRITE or ALL\r\n");
                    return;
                }
            };
            store.pause_clients(timeout, writes_only);
            output.extend_from_slice(b"+OK\r\n");
        } else if subcommand.eq_ignore_ascii_case(b"UNPAUSE") {
            if !rest.is_empty() {
                arity("client|unpause", output);
                return;
            }
            store.unpause_clients();
            output.extend_from_slice(b"+OK\r\n");
        } else if subcommand.eq_ignore_ascii_case(b"LIST") {
            self.client_list(store, rest, output);
        } else if subcommand.eq_ignore_ascii_case(b"KILL") {
            self.client_kill(store, rest, output);
        } else if subcommand.eq_ignore_ascii_case(b"INFO") && rest.is_empty() {
            let mut info = format!("id={} db=0", self.id).into_bytes();
            append_info(&mut info, b"name", self.name.as_deref());
            append_info(&mut info, b"lib-name", self.library_name.as_deref());
            append_info(&mut info, b"lib-ver", self.library_version.as_deref());
            if let Some(snapshot) = store.client_snapshot(self.id) {
                append_info(&mut info, b"addr", Some(snapshot.addr.as_bytes()));
                append_info(&mut info, b"laddr", Some(snapshot.laddr.as_bytes()));
                append_info(
                    &mut info,
                    b"resp",
                    Some(if snapshot.resp3 { b"3" } else { b"2" }),
                );
                append_info(&mut info, b"user", Some(&snapshot.user));
            }
            encode_bulk(Some(&info), output);
        } else {
            output
                .extend_from_slice(b"-ERR unsupported CLIENT subcommand or invalid arguments\r\n");
        }
    }

    pub(super) fn client_list(&self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let mut type_filter = None;
        let mut absent_type = false;
        let mut ids: Option<Vec<u64>> = None;
        let mut index = 0;
        while index < arguments.len() {
            if arguments[index].eq_ignore_ascii_case(b"TYPE") {
                let Some(value) = arguments.get(index + 1) else {
                    output.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                };
                if value.eq_ignore_ascii_case(b"normal") {
                    type_filter = Some(false);
                } else if value.eq_ignore_ascii_case(b"pubsub") {
                    type_filter = Some(true);
                } else if value.eq_ignore_ascii_case(b"master")
                    || value.eq_ignore_ascii_case(b"replica")
                    || value.eq_ignore_ascii_case(b"slave")
                {
                    // Standalone servers have no master/replica clients.
                    absent_type = true;
                } else {
                    output.extend_from_slice(b"-ERR unknown client type\r\n");
                    return;
                }
                index += 2;
            } else if arguments[index].eq_ignore_ascii_case(b"ID") {
                let mut parsed = Vec::new();
                for raw in &arguments[index + 1..] {
                    let Some(id) = parse_client_id(raw) else {
                        output.extend_from_slice(b"-ERR invalid client ID\r\n");
                        return;
                    };
                    parsed.push(id);
                }
                ids = Some(parsed);
                index = arguments.len();
            } else {
                output.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        let snapshots = if absent_type {
            Vec::new()
        } else {
            store.client_snapshots(type_filter)
        };
        let mut text = String::new();
        for snapshot in snapshots {
            if ids.as_ref().is_some_and(|ids| !ids.contains(&snapshot.id)) {
                continue;
            }
            let name = snapshot
                .name
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            let lib_name = snapshot
                .library_name
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            let lib_version = snapshot
                .library_version
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            let user = String::from_utf8_lossy(&snapshot.user);
            let mut flags = String::new();
            if snapshot.blocked {
                flags.push('b');
            }
            flags.push(if snapshot.sub + snapshot.psub > 0 {
                'P'
            } else {
                'N'
            });
            use std::fmt::Write as _;
            let _ = writeln!(
                text,
                "id={} addr={} laddr={} name={} age={} flags={} db=0 sub={} psub={} resp={} user={} lib-name={} lib-ver={}",
                snapshot.id,
                snapshot.addr,
                snapshot.laddr,
                name,
                snapshot.age_seconds,
                flags,
                snapshot.sub,
                snapshot.psub,
                if snapshot.resp3 { 3 } else { 2 },
                user,
                lib_name,
                lib_version
            );
        }
        encode_bulk(Some(text.as_bytes()), output);
    }

    pub(super) fn client_kill(&self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.is_empty() {
            arity("client|kill", output);
            return;
        }
        let legacy = arguments.len() == 1;
        let mut addr_filter: Option<Vec<u8>> = None;
        let mut laddr_filter: Option<Vec<u8>> = None;
        let mut id_filter = None;
        let mut type_filter = None;
        let mut absent_type = false;
        let mut skip_caller = true;
        let mut user_filter: Option<Vec<u8>> = None;
        let mut max_age = None;
        if legacy {
            addr_filter = Some(arguments[0].to_vec());
        } else {
            let mut index = 0;
            while index < arguments.len() {
                let Some(value) = arguments.get(index + 1) else {
                    output.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                };
                if arguments[index].eq_ignore_ascii_case(b"ID") {
                    let Some(id) = parse_client_id(value) else {
                        output.extend_from_slice(b"-ERR invalid client ID\r\n");
                        return;
                    };
                    id_filter = Some(id);
                } else if arguments[index].eq_ignore_ascii_case(b"ADDR") {
                    addr_filter = Some(value.to_vec());
                } else if arguments[index].eq_ignore_ascii_case(b"LADDR") {
                    laddr_filter = Some(value.to_vec());
                } else if arguments[index].eq_ignore_ascii_case(b"TYPE") {
                    if value.eq_ignore_ascii_case(b"normal") {
                        type_filter = Some(false);
                    } else if value.eq_ignore_ascii_case(b"pubsub") {
                        type_filter = Some(true);
                    } else if value.eq_ignore_ascii_case(b"master")
                        || value.eq_ignore_ascii_case(b"replica")
                        || value.eq_ignore_ascii_case(b"slave")
                    {
                        absent_type = true;
                    } else {
                        output.extend_from_slice(b"-ERR unknown client type\r\n");
                        return;
                    }
                } else if arguments[index].eq_ignore_ascii_case(b"USER") {
                    user_filter = Some(value.to_vec());
                } else if arguments[index].eq_ignore_ascii_case(b"MAXAGE") {
                    let Some(age) = std::str::from_utf8(value)
                        .ok()
                        .and_then(|value| value.parse::<u64>().ok())
                    else {
                        output.extend_from_slice(b"-ERR invalid client MAXAGE\r\n");
                        return;
                    };
                    max_age = Some(age);
                } else if arguments[index].eq_ignore_ascii_case(b"SKIPME") {
                    if value.eq_ignore_ascii_case(b"yes") {
                        skip_caller = true;
                    } else if value.eq_ignore_ascii_case(b"no") {
                        skip_caller = false;
                    } else {
                        output.extend_from_slice(b"-ERR syntax error\r\n");
                        return;
                    }
                } else {
                    output.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                index += 2;
            }
        }
        let mut ids = Vec::new();
        let snapshots = if absent_type {
            Vec::new()
        } else {
            store.client_snapshots(type_filter)
        };
        for snapshot in snapshots {
            if id_filter.is_some_and(|id| snapshot.id != id) {
                continue;
            }
            if addr_filter
                .as_deref()
                .is_some_and(|addr| snapshot.addr.as_bytes() != addr)
            {
                continue;
            }
            if laddr_filter
                .as_deref()
                .is_some_and(|addr| snapshot.laddr.as_bytes() != addr)
            {
                continue;
            }
            if user_filter
                .as_deref()
                .is_some_and(|user| snapshot.user.as_slice() != user)
            {
                continue;
            }
            if max_age.is_some_and(|age| snapshot.age_ms < age) {
                continue;
            }
            ids.push(snapshot.id);
        }
        let killed = store.client_kill_ids(&ids, if skip_caller { self.id } else { 0 });
        if legacy {
            if killed == 0 {
                output.extend_from_slice(b"-ERR No such client\r\n");
            } else {
                output.extend_from_slice(b"+OK\r\n");
            }
        } else {
            let _ = write!(output, ":{killed}\r\n");
        }
    }
    pub(super) fn set_name(&mut self, name: &[u8]) {
        self.name = (!name.is_empty()).then(|| name.to_vec());
    }
}

pub(super) fn parse_client_id(raw: &[u8]) -> Option<u64> {
    if raw.is_empty() || !raw.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(raw).ok()?.parse().ok()
}

pub(super) fn valid_metadata(bytes: &[u8], maximum: usize) -> bool {
    bytes.len() <= maximum && bytes.iter().all(|byte| (b'!'..=b'~').contains(byte))
}

pub(super) fn append_info(output: &mut Vec<u8>, key: &[u8], value: Option<&[u8]>) {
    if let Some(value) = value {
        output.push(b' ');
        output.extend_from_slice(key);
        output.push(b'=');
        output.extend_from_slice(value);
    }
}

pub(super) fn select(arguments: &[&[u8]], output: &mut Vec<u8>) {
    if arguments.len() != 1
        || arguments[0].is_empty()
        || !arguments[0].iter().all(u8::is_ascii_digit)
    {
        output.extend_from_slice(b"-ERR invalid DB index\r\n");
    } else if arguments[0].iter().all(|byte| *byte == b'0') {
        output.extend_from_slice(b"+OK\r\n");
    } else {
        output.extend_from_slice(b"-ERR DB index is out of range\r\n");
    }
}
