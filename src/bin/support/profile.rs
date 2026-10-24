//! Opt-in Linux procfs accounting. No subprocesses, FFI or assumed clock frequency.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, read};
use std::io;
use std::time::Instant;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn number(bytes: &[u8]) -> io::Result<u64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return Err(invalid("invalid unsigned procfs counter"));
    }
    std::str::from_utf8(bytes)
        .map_err(|_| invalid("non-ASCII procfs counter"))?
        .parse()
        .map_err(|_| invalid("procfs counter overflow"))
}

fn words(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(u8::is_ascii_whitespace)
        .filter(|s| !s.is_empty())
}

#[derive(Clone, Copy, Debug)]
struct Stat {
    pid: u64,
    state: u8,
    user_ticks: u64,
    system_ticks: u64,
    start_ticks: u64,
}

fn parse_stat(bytes: &[u8]) -> io::Result<Stat> {
    // comm can contain spaces, parentheses, newlines and non-UTF-8 bytes.
    let open = bytes.iter().position(|&c| c == b'(');
    let close = bytes.iter().rposition(|&c| c == b')');
    let (Some(open), Some(close)) = (open, close) else {
        return Err(invalid("missing procfs stat command delimiters"));
    };
    if open >= close {
        return Err(invalid("invalid procfs stat command delimiters"));
    }
    let mut prefix = words(&bytes[..open]);
    let pid = number(prefix.next().ok_or_else(|| invalid("missing stat PID"))?)?;
    if pid == 0 || prefix.next().is_some() {
        return Err(invalid("invalid stat PID"));
    }
    let fields: Vec<_> = words(&bytes[close + 1..]).collect();
    if fields.len() < 20 || fields[0].len() != 1 || !b"RSDZTtWXxKPI".contains(&fields[0][0]) {
        return Err(invalid("truncated procfs stat"));
    }
    Ok(Stat {
        pid,
        state: fields[0][0],
        user_ticks: number(fields[11])?, // field 14, following comm (field 2)
        system_ticks: number(fields[12])?, // field 15
        start_ticks: number(fields[19])?, // field 22
    })
}

fn field(bytes: &[u8], key: &[u8], kilobytes: bool) -> io::Result<u64> {
    let mut found = None;
    for line in bytes.split(|&c| c == b'\n') {
        if let Some(value) = line.strip_prefix(key) {
            if found.is_some() {
                return Err(invalid("duplicate procfs status field"));
            }
            let mut values = words(value);
            let mut value = number(values.next().ok_or_else(|| invalid("empty status field"))?)?;
            if kilobytes {
                if values.next() != Some(b"kB".as_slice()) {
                    return Err(invalid("expected procfs kB unit"));
                }
                value = value
                    .checked_mul(1024)
                    .ok_or_else(|| invalid("RSS overflow"))?;
            }
            if values.next().is_some() {
                return Err(invalid("extra procfs status field values"));
            }
            found = Some(value);
        }
    }
    found.ok_or_else(|| invalid("missing required procfs status field"))
}

#[derive(Clone, Copy, Debug)]
struct Memory {
    rss_bytes: u64,
    lifetime_peak_rss_bytes: u64,
    threads: u64,
}

fn parse_memory(bytes: &[u8]) -> io::Result<Memory> {
    let memory = Memory {
        rss_bytes: field(bytes, b"VmRSS:", true)?,
        lifetime_peak_rss_bytes: field(bytes, b"VmHWM:", true)?,
        threads: field(bytes, b"Threads:", false)?,
    };
    if memory.threads == 0 {
        return Err(invalid("process reports no threads"));
    }
    Ok(memory)
}

#[derive(Clone, Copy, Debug)]
struct Switches {
    voluntary: u64,
    involuntary: u64,
}

fn parse_switches(bytes: &[u8]) -> io::Result<Switches> {
    Ok(Switches {
        voluntary: field(bytes, b"voluntary_ctxt_switches:", false)?,
        involuntary: field(bytes, b"nonvoluntary_ctxt_switches:", false)?,
    })
}

fn clock_ticks(bytes: &[u8]) -> io::Result<u64> {
    // Linux auxv uses native unsigned-long pairs. For the supported Rust Linux
    // ABIs unsigned long has usize's width; no pointers are read/dereferenced.
    const WORD: usize = size_of::<usize>();
    let (pairs, remainder) = bytes.as_chunks::<{ 2 * WORD }>();
    if !remainder.is_empty() {
        return Err(invalid("truncated Linux auxiliary vector"));
    }
    let mut ticks = None;
    for pair in pairs {
        let key = usize::from_ne_bytes(pair[..WORD].try_into().expect("word-sized slice"));
        let value = usize::from_ne_bytes(pair[WORD..].try_into().expect("word-sized slice"));
        match key {
            0 => {
                if value != 0 {
                    return Err(invalid("invalid auxiliary vector terminator"));
                }
                return ticks.ok_or_else(|| invalid("AT_CLKTCK missing from auxiliary vector"));
            }
            17 => {
                if value == 0 || ticks.is_some() {
                    return Err(invalid("invalid or duplicate AT_CLKTCK"));
                }
                ticks = Some(u64::try_from(value).map_err(|_| invalid("AT_CLKTCK overflow"))?);
            }
            _ => {}
        }
    }
    Err(invalid("unterminated Linux auxiliary vector"))
}

fn live(stat: Stat, pid: u32, start_ticks: u64) -> io::Result<()> {
    if stat.pid != u64::from(pid)
        || stat.start_ticks != start_ticks
        || matches!(stat.state, b'Z' | b'X' | b'x')
    {
        return Err(invalid("profile target exited or its PID was reused"));
    }
    Ok(())
}

pub(super) struct Profiler {
    pid: u32,
    start_ticks: u64,
    clock_ticks_per_second: u64,
}

pub(super) struct Snapshot {
    at: Instant,
    stat: Stat,
    memory: Memory,
    // A TID alone can be reused; match the thread's start tick too.
    tasks: BTreeMap<(u64, u64), Switches>,
    raced: bool,
}

impl Profiler {
    pub(super) fn new(pid: u32) -> io::Result<Self> {
        if !cfg!(target_os = "linux") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "profiling requires Linux procfs",
            ));
        }
        let clock_ticks_per_second = clock_ticks(&read("/proc/self/auxv")?)?;
        let stat = parse_stat(&read(format!("/proc/{pid}/stat"))?)?;
        live(stat, pid, stat.start_ticks)?;
        Ok(Self {
            pid,
            start_ticks: stat.start_ticks,
            clock_ticks_per_second,
        })
    }

    pub(super) fn snapshot(&self) -> io::Result<Snapshot> {
        let root = format!("/proc/{}", self.pid);
        let at = Instant::now();
        // /proc/PID/stat CPU time is the process/thread-group total, including
        // exited threads, NOT merely the leader's /proc/PID/task/PID/stat time.
        let stat = parse_stat(&read(format!("{root}/stat"))?)?;
        live(stat, self.pid, self.start_ticks)?;
        let memory = parse_memory(&read(format!("{root}/status"))?)?;
        let mut tasks = BTreeMap::new();
        let mut raced = false;
        for entry in fs::read_dir(format!("{root}/task"))? {
            let entry = entry?;
            let tid = number(entry.file_name().as_encoded_bytes())?;
            let path = entry.path();
            let sample = (|| -> io::Result<_> {
                let before = parse_stat(&read(path.join("stat"))?)?;
                let switches = parse_switches(&read(path.join("status"))?)?;
                let after = parse_stat(&read(path.join("stat"))?)?;
                if before.pid != tid || after.pid != tid || before.start_ticks != after.start_ticks
                {
                    return Ok(None);
                }
                Ok(Some(((tid, before.start_ticks), switches)))
            })();
            match sample {
                Ok(Some((identity, switches))) => {
                    tasks.insert(identity, switches);
                }
                Ok(None) => raced = true,
                // A disappearing task can return ENOENT or ESRCH (Linux errno 3).
                // Other errors, including permissions and malformed data, fail.
                Err(e) if e.kind() == io::ErrorKind::NotFound || e.raw_os_error() == Some(3) => {
                    raced = true
                }
                Err(e) => return Err(e),
            }
        }
        raced |= tasks.len() as u64 != memory.threads;
        live(
            parse_stat(&read(format!("{root}/stat"))?)?,
            self.pid,
            self.start_ticks,
        )?;
        Ok(Snapshot {
            at,
            stat,
            memory,
            tasks,
            raced,
        })
    }

    pub(super) fn measure(&self, before: Snapshot, after: Snapshot) -> io::Result<Measurement> {
        live(before.stat, self.pid, self.start_ticks)?;
        live(after.stat, self.pid, self.start_ticks)?;
        let seconds = after
            .at
            .checked_duration_since(before.at)
            .filter(|d| !d.is_zero())
            .ok_or_else(|| invalid("invalid profiling observation interval"))?
            .as_secs_f64();
        let user_ticks = delta(before.stat.user_ticks, after.stat.user_ticks)?;
        let system_ticks = delta(before.stat.system_ticks, after.stat.system_ticks)?;
        let mut voluntary = 0u64;
        let mut involuntary = 0u64;
        let mut matched = 0;
        for (identity, first) in &before.tasks {
            if let Some(last) = after.tasks.get(identity) {
                voluntary = voluntary
                    .checked_add(delta(first.voluntary, last.voluntary)?)
                    .ok_or_else(|| invalid("context switch sum overflow"))?;
                involuntary = involuntary
                    .checked_add(delta(first.involuntary, last.involuntary)?)
                    .ok_or_else(|| invalid("context switch sum overflow"))?;
                matched += 1;
            }
        }
        let thread_set_unchanged = !before.raced
            && !after.raced
            && matched == before.tasks.len()
            && matched == after.tasks.len();
        Ok(Measurement {
            pid: self.pid,
            process_start_ticks: self.start_ticks,
            clock_ticks_per_second: self.clock_ticks_per_second,
            seconds,
            user_ticks,
            system_ticks,
            memory_before: before.memory,
            memory_after: after.memory,
            sampled_threads_before: before.tasks.len(),
            sampled_threads_after: after.tasks.len(),
            matched,
            thread_set_unchanged,
            voluntary,
            involuntary,
        })
    }
}

fn delta(before: u64, after: u64) -> io::Result<u64> {
    after
        .checked_sub(before)
        .ok_or_else(|| invalid("profiling counter decreased"))
}

pub(super) struct Measurement {
    pid: u32,
    process_start_ticks: u64,
    clock_ticks_per_second: u64,
    seconds: f64,
    user_ticks: u64,
    system_ticks: u64,
    memory_before: Memory,
    memory_after: Memory,
    sampled_threads_before: usize,
    sampled_threads_after: usize,
    matched: usize,
    thread_set_unchanged: bool,
    voluntary: u64,
    involuntary: u64,
}

impl Measurement {
    pub(super) fn json(&self) -> String {
        let user_seconds = self.user_ticks as f64 / self.clock_ticks_per_second as f64;
        let system_seconds = self.system_ticks as f64 / self.clock_ticks_per_second as f64;
        let mut out = String::new();
        write!(out, "{{\"source\":\"linux-procfs\",\"pid\":{},\"process_start_ticks\":{},\"clock_ticks_per_second\":{},\"observation_seconds\":{:.9},\"user_ticks\":{},\"system_ticks\":{},\"user_seconds\":{:.9},\"system_seconds\":{:.9},\"cpu_percent_one_logical_cpu\":{:.3},\"rss_bytes_before\":{},\"rss_bytes_after\":{},\"lifetime_peak_rss_bytes\":{},\"threads_before\":{},\"threads_after\":{},\"sampled_threads_before\":{},\"sampled_threads_after\":{},\"matched_threads\":{},\"thread_set_unchanged\":{},\"context_switches_scope\":\"matched_thread_lifetimes\",\"voluntary_context_switches\":{},\"involuntary_context_switches\":{}}}",
            self.pid, self.process_start_ticks, self.clock_ticks_per_second, self.seconds, self.user_ticks, self.system_ticks,
            user_seconds, system_seconds, (user_seconds + system_seconds) / self.seconds * 100.0,
            self.memory_before.rss_bytes, self.memory_after.rss_bytes,
            self.memory_after.lifetime_peak_rss_bytes, self.memory_before.threads, self.memory_after.threads,
            self.sampled_threads_before, self.sampled_threads_after, self.matched,
            self.thread_set_unchanged, self.voluntary, self.involuntary).expect("String write");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn stat_bytes(name: &[u8]) -> Vec<u8> {
        let mut bytes = b"42 (".to_vec();
        bytes.extend_from_slice(name);
        // Fields 3..22. Child CPU times are intentionally very different.
        bytes.extend_from_slice(b") S 1 2 3 0 -1 0 0 0 0 0 120 30 9000 8000 20 0 2 0 456\n");
        bytes
    }

    #[test]
    fn stat_parsing_handles_arbitrary_comm_and_excludes_child_cpu() {
        for name in [b"server".as_slice(), b"a ) (\n\xff name"] {
            let stat = parse_stat(&stat_bytes(name)).unwrap();
            assert_eq!(
                (
                    stat.pid,
                    stat.user_ticks,
                    stat.system_ticks,
                    stat.start_ticks
                ),
                (42, 120, 30, 456)
            );
        }
        for bytes in [
            b"".as_slice(),
            b"42 () S",
            b"42 ) ( S",
            b"no () S",
            b"0 () S",
        ] {
            assert!(parse_stat(bytes).is_err());
        }
        let bytes = stat_bytes(b"server");
        for end in 0..bytes.len() - 4 {
            assert!(parse_stat(&bytes[..end]).is_err(), "truncation at {end}");
        }
    }

    #[test]
    fn arbitrary_procfs_bytes_never_panic() {
        let mut random = 7u32;
        for length in 0..1024 {
            let bytes: Vec<_> = (0..length % 257)
                .map(|_| {
                    random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                    (random >> 24) as u8
                })
                .collect();
            let _ = parse_stat(&bytes);
            let _ = parse_memory(&bytes);
            let _ = parse_switches(&bytes);
            let _ = clock_ticks(&bytes);
        }
        let original = stat_bytes(b"arbitrary ) ( name");
        for index in 0..original.len() {
            for value in [0, 255, b' ', b'\n', b')', b'('] {
                let mut bytes = original.clone();
                bytes[index] = value;
                let _ = parse_stat(&bytes);
            }
        }
    }

    #[test]
    fn status_requires_correct_units_fields_and_checked_numbers() {
        let status = b"Name:\t\xff(name)\nVmRSS:\t12 kB\nVmHWM: 20 kB\nThreads: 2\nvoluntary_ctxt_switches: 10\nnonvoluntary_ctxt_switches: 3\n";
        let memory = parse_memory(status).unwrap();
        assert_eq!(
            (
                memory.rss_bytes,
                memory.lifetime_peak_rss_bytes,
                memory.threads
            ),
            (12288, 20480, 2)
        );
        let switches = parse_switches(status).unwrap();
        assert_eq!((switches.voluntary, switches.involuntary), (10, 3));
        for bytes in [
            b"".as_slice(),
            b"VmRSS: 1 MB",
            b"VmRSS: -1 kB",
            b"VmRSS: 1 kB extra",
            b"VmRSS: 18446744073709551615 kB",
            b"VmRSS: 1 kB\nVmRSS: 2 kB",
        ] {
            assert!(field(bytes, b"VmRSS:", true).is_err());
        }
        for bytes in [
            b"".as_slice(),
            b"-1",
            b"+1",
            b"18446744073709551616",
            b"\xff",
            b" 1",
        ] {
            assert!(number(bytes).is_err());
        }
        assert_eq!(number(b"18446744073709551615").unwrap(), u64::MAX);
        assert!(parse_switches(b"voluntary_ctxt_switches: 1").is_err());
        assert!(parse_memory(b"VmRSS: 1 kB\nVmHWM: 1 kB\nThreads: 0").is_err());
    }

    fn auxv(entries: &[(usize, usize)]) -> Vec<u8> {
        entries
            .iter()
            .flat_map(|&(key, value)| [key.to_ne_bytes(), value.to_ne_bytes()].concat())
            .collect()
    }

    #[test]
    fn auxv_clock_is_native_width_and_never_guessed() {
        assert_eq!(
            clock_ticks(&auxv(&[(6, 4096), (17, 250), (0, 0)])).unwrap(),
            250
        );
        for entries in [
            vec![],
            vec![(0, 0)],
            vec![(17, 0), (0, 0)],
            vec![(17, 100)],
            vec![(17, 100), (17, 100), (0, 0)],
            vec![(17, 100), (0, 1)],
        ] {
            assert!(clock_ticks(&auxv(&entries)).is_err());
        }
        let mut bytes = auxv(&[(17, 100), (0, 0)]);
        bytes.pop();
        assert!(clock_ticks(&bytes).is_err());
    }

    fn profiler() -> Profiler {
        Profiler {
            pid: 42,
            start_ticks: 456,
            clock_ticks_per_second: 100,
        }
    }

    fn snapshot(at: Instant, ticks: u64, switches: u64) -> Snapshot {
        Snapshot {
            at,
            stat: Stat {
                pid: 42,
                state: b'S',
                user_ticks: ticks,
                system_ticks: ticks,
                start_ticks: 456,
            },
            memory: Memory {
                rss_bytes: 1024,
                lifetime_peak_rss_bytes: 2048,
                threads: 2,
            },
            tasks: BTreeMap::from([
                (
                    (42, 456),
                    Switches {
                        voluntary: switches,
                        involuntary: switches,
                    },
                ),
                (
                    (43, 457),
                    Switches {
                        voluntary: switches + 10,
                        involuntary: switches + 20,
                    },
                ),
            ]),
            raced: false,
        }
    }

    #[test]
    fn accounting_matches_all_thread_lifetimes_and_uses_observation_wall_time() {
        let at = Instant::now();
        let measurement = profiler()
            .measure(
                snapshot(at, 100, 20),
                snapshot(at + Duration::from_secs(2), 125, 30),
            )
            .unwrap();
        assert_eq!(
            (
                measurement.voluntary,
                measurement.involuntary,
                measurement.matched
            ),
            (20, 20, 2)
        );
        assert!(measurement.thread_set_unchanged);
        let json = measurement.json();
        assert!(json.contains("\"user_seconds\":0.250000000"));
        assert!(json.contains("\"cpu_percent_one_logical_cpu\":25.000"));
        assert!(json.contains("\"lifetime_peak_rss_bytes\":2048"));
    }

    #[test]
    fn task_churn_and_snapshot_races_are_not_claimed_as_complete_process_counts() {
        let at = Instant::now();
        for change in 0..3 {
            let before = snapshot(at, 100, 20);
            let mut after = snapshot(at + Duration::from_secs(1), 101, 30);
            match change {
                0 => {
                    after.tasks.remove(&(43, 457));
                }
                1 => {
                    after.tasks.remove(&(43, 457));
                    after.tasks.insert(
                        (43, 999),
                        Switches {
                            voluntary: 9999,
                            involuntary: 9999,
                        },
                    );
                }
                _ => after.raced = true,
            }
            let measurement = profiler().measure(before, after).unwrap();
            assert!(!measurement.thread_set_unchanged);
            assert_eq!(measurement.voluntary, if change == 2 { 20 } else { 10 });
            assert!(
                measurement
                    .json()
                    .contains("\"context_switches_scope\":\"matched_thread_lifetimes\"")
            );
        }
    }

    #[test]
    fn context_counter_sum_overflow_is_rejected() {
        let at = Instant::now();
        let mut before = snapshot(at, 100, 20);
        let mut after = snapshot(at + Duration::from_secs(1), 101, 30);
        for task in before.tasks.values_mut() {
            task.voluntary = 0;
        }
        for task in after.tasks.values_mut() {
            task.voluntary = u64::MAX;
        }
        assert!(profiler().measure(before, after).is_err());
    }

    #[test]
    fn target_exit_reuse_counter_regression_and_zero_intervals_fail() {
        let at = Instant::now();
        for change in 0..8 {
            let before = snapshot(at, 100, 20);
            let mut after = snapshot(at + Duration::from_secs(1), 101, 30);
            match change {
                0 => after.stat.start_ticks += 1,
                1 => after.stat.pid += 1,
                2 => after.stat.state = b'Z',
                3 => after.stat.user_ticks = 99,
                4 => after.at = at,
                5 => after.tasks.get_mut(&(43, 457)).unwrap().voluntary = 0,
                6 => after.at = at - Duration::from_secs(1),
                _ => after.stat.system_ticks = 99,
            }
            assert!(profiler().measure(before, after).is_err(), "case {change}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_procfs_sampling_and_missing_targets() {
        let profiler = Profiler::new(std::process::id()).unwrap();
        let first = profiler.snapshot().unwrap();
        let second = profiler.snapshot().unwrap();
        let measurement = profiler.measure(first, second).unwrap();
        assert!(measurement.clock_ticks_per_second > 0);
        assert!(measurement.sampled_threads_before > 0);
        assert!(measurement.memory_after.rss_bytes > 0);
        assert!(Profiler::new(u32::MAX).is_err());
    }
}
