//! Command routing, authorization ordering and RESP3 reply adaptation.

use std::io::Write;

use crate::engine::{Access, Action, Store, WatchState, XReadGroupStart, parse_xreadgroup};

use super::Protocol;
use super::Session;
use super::client::select;
use super::command::{allows_subscriber_command, command_name, is_auth_exempt};
use super::reply::convert_resp2_reply;

impl Session {
    pub(super) fn execute_inner(
        &mut self,
        store: &Store,
        arguments: &[&[u8]],
        output: &mut Vec<u8>,
    ) -> Action {
        let Some(command) = arguments.first() else {
            output.extend_from_slice(b"-ERR empty command\r\n");
            return Action::Continue;
        };
        store.record_command();
        store.monitor_command(self.id, arguments);
        if command.eq_ignore_ascii_case(b"RESET") {
            return self.reset(store, arguments, output);
        }
        if !is_auth_exempt(command) {
            let authentication_required = store.authentication_required();
            if authentication_required && self.authenticated_user.is_none() {
                self.log_denial(store, "noauth", "toplevel", command);
                output.extend_from_slice(b"-NOAUTH Authentication required.\r\n");
                return Action::Continue;
            }
            if authentication_required
                || store.acl_restricted()
                || self.authenticated_user.is_some()
            {
                match store.authorize(self.authenticated_user.as_deref(), arguments) {
                    Access::Allowed => {}
                    Access::NoAuth => {
                        self.log_denial(store, "noauth", "toplevel", command);
                        output.extend_from_slice(b"-NOAUTH Authentication required.\r\n");
                        return Action::Continue;
                    }
                    Access::DeniedCommand => {
                        let name = command_name(command);
                        self.log_denial(store, "command", "toplevel", name.as_bytes());
                        let _ = write!(
                            output,
                            "-NOPERM this user has no permissions to run the '{name}' command\r\n"
                        );
                        if let Some(multi) = self.multi.as_mut() {
                            multi.dirty = true;
                        }
                        return Action::Continue;
                    }
                    Access::DeniedKey(key) => {
                        self.log_denial(store, "key", "toplevel", &key);
                        output.extend_from_slice(
                            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
                        );
                        if let Some(multi) = self.multi.as_mut() {
                            multi.dirty = true;
                        }
                        return Action::Continue;
                    }
                    Access::DeniedChannel(channel) => {
                        self.log_denial(store, "channel", "toplevel", &channel);
                        output.extend_from_slice(
                            b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n",
                        );
                        if let Some(multi) = self.multi.as_mut() {
                            multi.dirty = true;
                        }
                        return Action::Continue;
                    }
                }
            }
        }
        let subscribed = self.is_subscribed();
        if subscribed && self.protocol == Protocol::Resp2 && !allows_subscriber_command(command) {
            let name = command_name(command);
            let _ = write!(
                output,
                "-ERR Can't execute '{name}': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n"
            );
            return Action::Continue;
        }
        // Ordinary data requests need none of the cold connection-control
        // probes below. Authorization/subscriber restrictions still run first;
        // MULTI queueing and RESP2 subscriber PING keep their original paths.
        if self.multi.is_none()
            && (!subscribed || self.protocol != Protocol::Resp2)
            && (command.eq_ignore_ascii_case(b"PING")
                || command.eq_ignore_ascii_case(b"GET")
                || command.eq_ignore_ascii_case(b"SET")
                || command.eq_ignore_ascii_case(b"ECHO"))
        {
            return self.execute_store(store, arguments, output);
        }
        if command.eq_ignore_ascii_case(b"MULTI") {
            self.multi(arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"EXEC") {
            let start = output.len();
            let action = self.exec(store, arguments, output);
            self.convert_reply(start, output);
            return action;
        }
        if command.eq_ignore_ascii_case(b"DISCARD") {
            self.discard(arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"WATCH") {
            self.watch(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"UNWATCH") {
            self.unwatch(arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"QUIT") {
            // QUIT executes immediately even inside MULTI, as in Redis.
            self.multi = None;
            self.watched = WatchState::default();
            self.pubsub_channels.clear();
            self.pubsub_patterns.clear();
            self.pubsub_shard_channels.clear();
            self.blocked = None;
            self.sync_blocked(store);
        } else if self.multi.is_some() {
            self.queue(arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"MONITOR") {
            self.monitor(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"WAITAOF") {
            self.waitaof(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"SHUTDOWN") {
            store.shutdown_command(arguments, output);
            return if store.shutdown_requested() {
                Action::Close
            } else {
                Action::Continue
            };
        }
        if command.eq_ignore_ascii_case(b"SUBSCRIBE") {
            self.subscribe(store, &arguments[1..], false, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"PSUBSCRIBE") {
            self.subscribe(store, &arguments[1..], true, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"UNSUBSCRIBE") {
            self.unsubscribe(store, &arguments[1..], false, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"PUNSUBSCRIBE") {
            self.unsubscribe(store, &arguments[1..], true, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"SSUBSCRIBE") {
            self.subscribe_shard(store, &arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"SUNSUBSCRIBE") {
            self.unsubscribe_shard(store, &arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"BLPOP") || command.eq_ignore_ascii_case(b"BRPOP") {
            let left = command.eq_ignore_ascii_case(b"BLPOP");
            self.blocking(store, &arguments[1..], left, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"BLMPOP") {
            self.blocking_lmpop(store, &arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"BLMOVE") || command.eq_ignore_ascii_case(b"BRPOPLPUSH") {
            self.blocking_list_move(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"BZPOPMIN") || command.eq_ignore_ascii_case(b"BZPOPMAX") {
            let max = command.eq_ignore_ascii_case(b"BZPOPMAX");
            self.blocking_zset_single(store, &arguments[1..], max, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"BZMPOP") {
            self.blocking_zpop(store, &arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"XREAD") {
            self.xread(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"XREADGROUP")
            && let Ok(options) = parse_xreadgroup(arguments)
            && options.block.is_some()
            && options
                .starts
                .iter()
                .all(|start| matches!(start, XReadGroupStart::New))
        {
            self.blocking_xreadgroup(store, &options, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"PUBLISH") {
            self.publish(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"SPUBLISH") {
            self.spublish(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"PUBSUB") {
            self.pubsub(store, &arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"PING")
            && self.is_subscribed()
            && self.protocol == Protocol::Resp2
        {
            self.subscriber_ping(&arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"HELLO") {
            self.hello(store, &arguments[1..], output);
            self.refresh_pubsub_protocol(store);
            self.sync_client_metadata(store);
            return if self.close_requested {
                Action::Close
            } else {
                Action::Continue
            };
        }
        if command.eq_ignore_ascii_case(b"CLIENT") {
            self.client(store, &arguments[1..], output);
            self.sync_client_metadata(store);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"SELECT") {
            select(&arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"AUTH") {
            self.auth(store, &arguments[1..], output);
            self.sync_client_metadata(store);
            return if self.close_requested {
                Action::Close
            } else {
                Action::Continue
            };
        }
        if command.eq_ignore_ascii_case(b"ACL") {
            self.acl(store, &arguments[1..], output);
            return Action::Continue;
        }

        self.execute_store(store, arguments, output)
    }

    pub(super) fn execute_store(
        &self,
        store: &Store,
        arguments: &[&[u8]],
        output: &mut Vec<u8>,
    ) -> Action {
        let start = output.len();
        let action = store.execute(arguments, output);
        self.convert_reply(start, output);
        action
    }

    pub(super) fn convert_reply(&self, start: usize, output: &mut Vec<u8>) {
        if self.protocol == Protocol::Resp3 {
            let converted = convert_resp2_reply(&output[start..]);
            output.truncate(start);
            output.extend_from_slice(&converted);
        }
    }
}
