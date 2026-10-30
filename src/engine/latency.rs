//! Bounded LATENCY event monitoring.
//!
//! Only events this server actually measures are recorded: `command` duration
//! around a client command attempt and `expire-cycle` around bounded expiry
//! reclamation. There is no fork, AOF-fsync, eviction-cycle or replication
//! latency source, so those series are never fabricated.

#[cfg(test)]
use super::ShardExecutor;
use crate::protocol::encode_bulk;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::time::Duration;

pub(crate) const MAX_SAMPLES: usize = 160;
pub(crate) const DEFAULT_THRESHOLD_MS: u64 = 0;
const COMMAND_EVENT: &str = "command";
const EXPIRE_EVENT: &str = "expire-cycle";

#[derive(Default)]
pub(crate) struct State {
    series: HashMap<&'static str, Series>,
}

struct Series {
    samples: VecDeque<(i64, u64)>,
    max: u64,
}

impl super::ServerServices {
    pub(crate) fn latency_threshold_ms(&self) -> u64 {
        self.latency_monitor_threshold_ms.load(Ordering::Relaxed)
    }

    pub(crate) fn latency_enabled(&self) -> bool {
        self.latency_threshold_ms() > 0
    }

    pub(crate) fn set_latency_threshold_ms(&self, value: u64) {
        self.latency_monitor_threshold_ms
            .store(value, Ordering::Relaxed);
    }

    /// Record one measured event if it reached the configured millisecond
    /// threshold. The lock is taken only for an admitted sample.
    pub(crate) fn record_latency(&self, event: &'static str, elapsed: Duration) {
        let threshold = self.latency_threshold_ms();
        if threshold == 0 {
            return;
        }
        let millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        if millis < threshold {
            return;
        }
        let timestamp = self.clock.now_ms().div_euclid(1000);
        let Ok(mut state) = self.latency.write() else {
            return;
        };
        let series = state.series.entry(event).or_insert_with(|| Series {
            samples: VecDeque::new(),
            max: 0,
        });
        series.max = series.max.max(millis);
        series.samples.push_back((timestamp, millis));
        while series.samples.len() > MAX_SAMPLES {
            series.samples.pop_front();
        }
    }

    pub(crate) fn record_command_latency(&self, elapsed: Duration) {
        self.record_latency(COMMAND_EVENT, elapsed);
    }

    pub(crate) fn record_expire_latency(&self, elapsed: Duration) {
        self.record_latency(EXPIRE_EVENT, elapsed);
    }

    pub(super) fn execute_latency(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> bool {
        let Some(command) = arguments.first() else {
            return false;
        };
        if !command.eq_ignore_ascii_case(b"LATENCY") {
            return false;
        }
        let Some(subcommand) = arguments.get(1) else {
            output.extend_from_slice(b"-ERR wrong number of arguments for 'latency' command\r\n");
            return true;
        };
        if subcommand.eq_ignore_ascii_case(b"LATEST") {
            if arguments.len() != 2 {
                latency_syntax_error(subcommand, output);
                return true;
            }
            let Ok(state) = self.latency.read() else {
                super::internal_error(output);
                return true;
            };
            let mut events: Vec<&&'static str> = state.series.keys().collect();
            events.sort_unstable();
            write_array_len(events.len(), output);
            for event in events {
                let Some(series) = state.series.get(*event) else {
                    continue;
                };
                write_array_len(4, output);
                encode_bulk(Some(event.as_bytes()), output);
                let (timestamp, latest) = series.samples.back().copied().unwrap_or((0, 0));
                super::integer(timestamp, output);
                super::integer(i64::try_from(latest).unwrap_or(i64::MAX), output);
                super::integer(i64::try_from(series.max).unwrap_or(i64::MAX), output);
            }
        } else if subcommand.eq_ignore_ascii_case(b"HISTORY") {
            if arguments.len() != 3 {
                latency_syntax_error(subcommand, output);
                return true;
            }
            let Ok(state) = self.latency.read() else {
                super::internal_error(output);
                return true;
            };
            let samples = state
                .series
                .iter()
                .find(|(event, _)| event.as_bytes().eq_ignore_ascii_case(arguments[2]))
                .map(|(_, series)| &series.samples);
            write_array_len(samples.map(VecDeque::len).unwrap_or(0), output);
            if let Some(samples) = samples {
                for (timestamp, latency) in samples {
                    write_array_len(2, output);
                    super::integer(*timestamp, output);
                    super::integer(i64::try_from(*latency).unwrap_or(i64::MAX), output);
                }
            }
        } else if subcommand.eq_ignore_ascii_case(b"RESET") {
            let Ok(mut state) = self.latency.write() else {
                super::internal_error(output);
                return true;
            };
            let reset = if arguments.len() == 2 {
                let count = state.series.len();
                state.series.clear();
                count
            } else {
                let mut reset = 0;
                for event in &arguments[2..] {
                    let known = state
                        .series
                        .keys()
                        .find(|known| known.as_bytes().eq_ignore_ascii_case(event))
                        .copied();
                    if let Some(known) = known
                        && state.series.remove(known).is_some()
                    {
                        reset += 1;
                    }
                }
                reset
            };
            super::integer(i64::try_from(reset).unwrap_or(i64::MAX), output);
        } else if subcommand.eq_ignore_ascii_case(b"DOCTOR") {
            if arguments.len() != 2 {
                latency_syntax_error(subcommand, output);
                return true;
            }
            let report = self.latency_report();
            encode_bulk(Some(report.as_bytes()), output);
        } else if subcommand.eq_ignore_ascii_case(b"HELP") {
            if arguments.len() != 2 {
                latency_syntax_error(subcommand, output);
                return true;
            }
            const HELP: &[&[u8]] = &[
                b"DOCTOR",
                b"    Return a human readable latency analysis report.",
                b"HISTORY <event>",
                b"    Return time-latency samples for the <event> class.",
                b"LATEST",
                b"    Return the latest latency samples for all events.",
                b"RESET [<event> ...]",
                b"    Reset latency data of one or more <event> classes.",
                b"    (default: reset all data for all event classes)",
            ];
            write_array_len(HELP.len(), output);
            for line in HELP {
                encode_bulk(Some(line), output);
            }
        } else if subcommand.eq_ignore_ascii_case(b"GRAPH") {
            output.extend_from_slice(b"-ERR LATENCY GRAPH is unsupported; use LATENCY HISTORY\r\n");
        } else if subcommand.eq_ignore_ascii_case(b"HISTOGRAM") {
            output.extend_from_slice(
                b"-ERR LATENCY HISTOGRAM is unsupported; use LATENCY HISTORY\r\n",
            );
        } else {
            latency_syntax_error(subcommand, output);
        }
        true
    }

    fn latency_report(&self) -> String {
        let threshold = self.latency_threshold_ms();
        if threshold == 0 {
            return "Latency monitoring is disabled in this server.\n\
                    Use CONFIG SET latency-monitor-threshold <milliseconds> to enable it.\n\
                    Only command and expire-cycle events are measured; fork, AOF, \
                    eviction-cycle and replication latency are not available.\n"
                .to_string();
        }
        let Ok(state) = self.latency.read() else {
            return "Latency data is temporarily unavailable.\n".to_string();
        };
        if state.series.is_empty() {
            return format!(
                "Latency monitoring is enabled at {threshold} ms, but no measured event \
                 has reached the threshold.\nMonitored events: command, expire-cycle.\n"
            );
        }
        let mut events: Vec<&&'static str> = state.series.keys().collect();
        events.sort_unstable();
        let mut report = format!(
            "Latency monitoring threshold: {threshold} ms\n\
             Measured events:\n"
        );
        for event in events {
            if let Some(series) = state.series.get(*event) {
                let (_, latest) = series.samples.back().copied().unwrap_or((0, 0));
                report.push_str(&format!(
                    "  {event}: samples={} latest={latest} ms max={} ms\n",
                    series.samples.len(),
                    series.max
                ));
            }
        }
        report.push_str(
            "Advice: inspect the affected command or expiry cycle directly.\n\
             Only command and expire-cycle events are measured; fork, AOF, \
             eviction-cycle and replication latency are not available.\n",
        );
        report
    }
}

fn write_array_len(length: usize, output: &mut Vec<u8>) {
    use std::io::Write;
    let _ = write!(output, "*{length}\r\n");
}

fn latency_syntax_error(subcommand: &[u8], output: &mut Vec<u8>) {
    output.extend_from_slice(b"-ERR unknown subcommand or wrong number of arguments for 'latency|");
    output.extend_from_slice(subcommand);
    output.extend_from_slice(b"'. Try LATENCY HELP.\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_thresholded_bounded_and_report_latest_and_max() {
        let store = ShardExecutor::new(1024);
        assert!(!store.latency_enabled());
        store.record_latency(COMMAND_EVENT, Duration::from_millis(5));
        assert_eq!(store.latency.read().unwrap().series.len(), 0);

        store.set_latency_threshold_ms(2);
        store.record_latency(COMMAND_EVENT, Duration::from_millis(1));
        store.record_latency(COMMAND_EVENT, Duration::from_millis(3));
        store.record_latency(COMMAND_EVENT, Duration::from_millis(8));
        store.record_latency(EXPIRE_EVENT, Duration::from_millis(4));
        {
            let state = store.latency.read().unwrap();
            let command = &state.series[COMMAND_EVENT];
            assert_eq!(command.samples.len(), 2);
            assert_eq!(
                command.samples.back().copied().map(|sample| sample.1),
                Some(8)
            );
            assert_eq!(command.max, 8);
            assert_eq!(state.series[EXPIRE_EVENT].max, 4);
        }

        store.set_latency_threshold_ms(1);
        for _ in 0..MAX_SAMPLES + 5 {
            store.record_latency(COMMAND_EVENT, Duration::from_millis(1));
        }
        assert_eq!(
            store.latency.read().unwrap().series[COMMAND_EVENT]
                .samples
                .len(),
            MAX_SAMPLES
        );

        let mut output = Vec::new();
        assert!(store.execute_latency(&[b"LATENCY", b"RESET", b"command"], &mut output));
        assert_eq!(output, b":1\r\n");
        let mut output = Vec::new();
        assert!(store.execute_latency(&[b"LATENCY", b"RESET"], &mut output));
        assert_eq!(output, b":1\r\n");
        assert_eq!(store.latency.read().unwrap().series.len(), 0);
    }

    #[test]
    fn history_and_latest_use_resp2_shapes() {
        let store = ShardExecutor::new(1024);
        store.set_latency_threshold_ms(1);
        store.record_latency(COMMAND_EVENT, Duration::from_millis(2));
        let mut output = Vec::new();
        assert!(store.execute_latency(&[b"LATENCY", b"HISTORY", b"COMMAND"], &mut output));
        assert!(output.starts_with(b"*1\r\n*2\r\n"));
        let mut output = Vec::new();
        assert!(store.execute_latency(&[b"LATENCY", b"LATEST"], &mut output));
        assert!(output.starts_with(b"*1\r\n*4\r\n$7\r\ncommand\r\n"));
        let mut output = Vec::new();
        assert!(store.execute_latency(&[b"LATENCY", b"HISTORY", b"unknown"], &mut output));
        assert_eq!(output, b"*0\r\n");
        let mut output = Vec::new();
        assert!(store.execute_latency(&[b"LATENCY", b"GRAPH", b"command"], &mut output));
        assert!(output.starts_with(b"-ERR LATENCY GRAPH is unsupported"));
    }
}
