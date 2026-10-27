//! Explicit behavior for unimplemented cluster, module and replication features.
//!
//! These commands must not pretend that cluster routing, module loading or
//! replicas exist. They return bounded, documented responses so clients can
//! detect the standalone/unsupported boundary without treating the server as a
//! complete cluster or module host.

use super::options::NOT_INTEGER;
use super::{Store, arity, integer};
use crate::protocol::encode_bulk;
use std::io::Write as _;

const CLUSTER_DISABLED: &[u8] = b"-ERR This instance has cluster support disabled\r\n";
const MODULE_DISABLED: &[u8] = b"-ERR MODULE loading is not supported by this server\r\n";
const RDB_DISABLED: &[u8] = b"-ERR RDB persistence is not supported by this server\r\n";
const REPLICATION_DISABLED: &[u8] =
    b"-ERR replication is not supported by this standalone server\r\n";
const MULTI_DB_DISABLED: &[u8] = b"-ERR this server supports only database 0\r\n";

impl Store {
    pub(super) fn execute_unsupported(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let Some(command) = args.first() else {
            return false;
        };
        if command.eq_ignore_ascii_case(b"CLUSTER") {
            if args.len() < 2 {
                arity("cluster", out);
            } else if args[1].eq_ignore_ascii_case(b"HELP") {
                out.extend_from_slice(b"*1\r\n");
                encode_bulk(
                    Some(b"CLUSTER is unavailable: this standalone server has cluster support disabled."),
                    out,
                );
            } else {
                out.extend_from_slice(CLUSTER_DISABLED);
            }
        } else if command.eq_ignore_ascii_case(b"MODULE") {
            if args.len() < 2 {
                arity("module", out);
            } else if args[1].eq_ignore_ascii_case(b"LIST") && args.len() == 2 {
                out.extend_from_slice(b"*0\r\n");
            } else if args[1].eq_ignore_ascii_case(b"HELP") && args.len() == 2 {
                out.extend_from_slice(b"*1\r\n");
                encode_bulk(
                    Some(b"MODULE LIST is empty; loading modules is not supported by this server."),
                    out,
                );
            } else if (args[1].eq_ignore_ascii_case(b"LOAD")
                || args[1].eq_ignore_ascii_case(b"LOADEX")
                || args[1].eq_ignore_ascii_case(b"UNLOAD"))
                && args.len() >= 3
            {
                out.extend_from_slice(MODULE_DISABLED);
            } else {
                out.extend_from_slice(b"-ERR unknown MODULE subcommand\r\n");
            }
        } else if command.eq_ignore_ascii_case(b"WAIT") {
            if args.len() != 3 {
                arity("wait", out);
            } else if std::str::from_utf8(args[1])
                .ok()
                .and_then(|text| text.parse::<i64>().ok())
                .is_none()
            {
                out.extend_from_slice(NOT_INTEGER);
            } else {
                let Some(timeout) = std::str::from_utf8(args[2])
                    .ok()
                    .and_then(|text| text.parse::<i64>().ok())
                else {
                    out.extend_from_slice(b"-ERR timeout is not an integer or out of range\r\n");
                    return true;
                };
                if timeout < 0 {
                    out.extend_from_slice(b"-ERR timeout is negative\r\n");
                } else {
                    // No replicas are configured; no acknowledgement can succeed.
                    integer(0, out);
                }
            }
        } else if command.eq_ignore_ascii_case(b"WAITAOF") {
            // Queue-time blocking commands execute without blocking inside EXEC.
            if args.len() != 4 {
                arity("waitaof", out);
            } else {
                let parse = |raw: &[u8]| {
                    std::str::from_utf8(raw)
                        .ok()
                        .and_then(|text| text.parse::<i64>().ok())
                };
                let (Some(numlocal), Some(numreplicas), Some(timeout)) =
                    (parse(args[1]), parse(args[2]), parse(args[3]))
                else {
                    out.extend_from_slice(b"-ERR value is not an integer or out of range\r\n");
                    return true;
                };
                if timeout < 0 {
                    out.extend_from_slice(b"-ERR timeout can't be negative\r\n");
                } else {
                    match self.waitaof_probe(numlocal, numreplicas) {
                        Ok((local, replicas)) => {
                            let _ = write!(out, "*2\r\n:{local}\r\n:{replicas}\r\n");
                        }
                        Err(error) => out.extend_from_slice(error),
                    }
                }
            }
        } else if command.eq_ignore_ascii_case(b"MONITOR") {
            // Top-level MONITOR is handled by the session; this path only sees a
            // queued copy executed by EXEC, which cannot change session mode.
            out.extend_from_slice(b"-ERR MONITOR is not allowed inside MULTI\r\n");
        } else if command.eq_ignore_ascii_case(b"LASTSAVE") {
            if args.len() != 1 {
                arity("lastsave", out);
            } else {
                // No RDB save has ever completed because RDB persistence is absent.
                integer(0, out);
            }
        } else if command.eq_ignore_ascii_case(b"SAVE") {
            if args.len() != 1 {
                arity("save", out);
            } else {
                out.extend_from_slice(RDB_DISABLED);
            }
        } else if command.eq_ignore_ascii_case(b"BGSAVE") {
            if args.len() == 1 || (args.len() == 2 && args[1].eq_ignore_ascii_case(b"SCHEDULE")) {
                out.extend_from_slice(RDB_DISABLED);
            } else if args.len() == 2 {
                out.extend_from_slice(b"-ERR syntax error\r\n");
            } else {
                arity("bgsave", out);
            }
        } else if command.eq_ignore_ascii_case(b"REPLICAOF")
            || command.eq_ignore_ascii_case(b"SLAVEOF")
        {
            let name = if command.eq_ignore_ascii_case(b"REPLICAOF") {
                "replicaof"
            } else {
                "slaveof"
            };
            if args.len() != 3 {
                arity(name, out);
            } else if args[1].eq_ignore_ascii_case(b"NO") && args[2].eq_ignore_ascii_case(b"ONE") {
                // This server is always a standalone master, so promotion is a no-op.
                out.extend_from_slice(b"+OK\r\n");
            } else {
                out.extend_from_slice(REPLICATION_DISABLED);
            }
        } else if command.eq_ignore_ascii_case(b"PSYNC") {
            if args.len() < 3 {
                arity("psync", out);
            } else {
                out.extend_from_slice(REPLICATION_DISABLED);
            }
        } else if command.eq_ignore_ascii_case(b"SYNC") {
            if args.len() != 1 {
                arity("sync", out);
            } else {
                out.extend_from_slice(REPLICATION_DISABLED);
            }
        } else if command.eq_ignore_ascii_case(b"REPLCONF") {
            if args.len() < 2 {
                arity("replconf", out);
            } else {
                out.extend_from_slice(REPLICATION_DISABLED);
            }
        } else if command.eq_ignore_ascii_case(b"FAILOVER") {
            out.extend_from_slice(REPLICATION_DISABLED);
        } else if command.eq_ignore_ascii_case(b"MOVE") || command.eq_ignore_ascii_case(b"SWAPDB") {
            let name = if command.eq_ignore_ascii_case(b"MOVE") {
                "move"
            } else {
                "swapdb"
            };
            if args.len() != 3 {
                arity(name, out);
            } else {
                out.extend_from_slice(MULTI_DB_DISABLED);
            }
        } else if command.eq_ignore_ascii_case(b"READONLY")
            || command.eq_ignore_ascii_case(b"READWRITE")
            || command.eq_ignore_ascii_case(b"ASKING")
        {
            if args.len() != 1 {
                arity(
                    if command.eq_ignore_ascii_case(b"READONLY") {
                        "readonly"
                    } else if command.eq_ignore_ascii_case(b"READWRITE") {
                        "readwrite"
                    } else {
                        "asking"
                    },
                    out,
                );
            } else {
                out.extend_from_slice(CLUSTER_DISABLED);
            }
        } else {
            return false;
        }
        true
    }
}
