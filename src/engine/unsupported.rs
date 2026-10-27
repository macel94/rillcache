//! Explicit behavior for unimplemented cluster, module and replication features.
//!
//! These commands must not pretend that cluster routing, module loading or
//! replicas exist. They return bounded, documented responses so clients can
//! detect the standalone/unsupported boundary without treating the server as a
//! complete cluster or module host.

use super::options::NOT_INTEGER;
use super::{Store, arity, integer};
use crate::protocol::encode_bulk;

const CLUSTER_DISABLED: &[u8] = b"-ERR This instance has cluster support disabled\r\n";
const MODULE_DISABLED: &[u8] = b"-ERR MODULE loading is not supported by this server\r\n";

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
