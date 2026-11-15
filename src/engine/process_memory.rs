//! Safe process-memory observation for INFO and MEMORY STATS.
//!
//! Linux exposes the resident set in `/proc/self/status`. This is an
//! observation only: it includes allocator metadata, thread stacks, socket
//! buffers and shared pages, and is deliberately not used for payload
//! admission or `maxmemory` enforcement. Other platforms report unavailable.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProcessMemory {
    pub resident_bytes: u64,
}

/// Observational cgroup memory usage. `limit_bytes` is `None` when the cgroup
/// reports `max`/an effectively unlimited value; this is never used for
/// admission or eviction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CgroupMemory {
    pub current_bytes: u64,
    pub limit_bytes: Option<u64>,
}

pub(crate) fn snapshot() -> Option<ProcessMemory> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let resident_kib = parse_vm_rss_kib(&status)?;
    Some(ProcessMemory {
        resident_bytes: resident_kib.checked_mul(1024)?,
    })
}

/// Read the cgroup v2 (then v1) current/limit pair for this process. Missing
/// files or a `max` limit leave the corresponding value absent.
pub(crate) fn cgroup_snapshot() -> Option<CgroupMemory> {
    let root = cgroup_root();
    let v2 = root.join("memory.current");
    let v2_limit = root.join("memory.max");
    if let Some(current) = read_u64(&v2) {
        let limit = std::fs::read_to_string(&v2_limit).ok();
        return Some(CgroupMemory {
            current_bytes: current,
            limit_bytes: limit.as_deref().and_then(parse_cgroup_limit),
        });
    }
    let v1 = std::path::Path::new("/sys/fs/cgroup/memory");
    let current = read_u64(&v1.join("memory.usage_in_bytes"))?;
    let limit = std::fs::read_to_string(v1.join("memory.limit_in_bytes")).ok();
    Some(CgroupMemory {
        current_bytes: current,
        limit_bytes: limit.as_deref().and_then(parse_cgroup_limit),
    })
}

fn cgroup_root() -> std::path::PathBuf {
    let base = std::path::Path::new("/sys/fs/cgroup");
    let Ok(contents) = std::fs::read_to_string("/proc/self/cgroup") else {
        return base.to_path_buf();
    };
    for line in contents.lines() {
        let Some(rest) = line.strip_prefix("0::") else {
            continue;
        };
        let relative = rest.trim_start_matches('/');
        let candidate = base.join(relative);
        if candidate.join("memory.current").is_file() {
            return candidate;
        }
    }
    base.to_path_buf()
}

fn read_u64(path: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn parse_cgroup_limit(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if raw == "max" {
        return None;
    }
    let value = raw.parse::<u64>().ok()?;
    // cgroup v1 represents "unlimited" as a near-u64::MAX page-aligned value.
    (value < (1u64 << 60)).then_some(value)
}

fn parse_vm_rss_kib(status: &str) -> Option<u64> {
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?;
    value.parse::<u64>().ok()
}

/// Redis-compatible short binary-unit rendering, for example `0B`, `1.50K`.
pub(crate) fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else {
        format!("{value:.2}{}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_rss_is_parsed_from_proc_status() {
        let status = "Name:\trillcache\nVmPeak:\t 9000 kB\nVmRSS:\t    1234 kB\nThreads:\t2\n";
        assert_eq!(parse_vm_rss_kib(status), Some(1234));
        assert_eq!(parse_vm_rss_kib("VmRSS:\tnot-a-number kB\n"), None);
        assert_eq!(parse_vm_rss_kib("Name:\trillcache\n"), None);
    }

    #[test]
    fn human_units_are_short_and_binary() {
        assert_eq!(human_bytes(0), "0B");
        assert_eq!(human_bytes(1023), "1023B");
        assert_eq!(human_bytes(1024), "1.00K");
        assert_eq!(human_bytes(1536), "1.50K");
        assert_eq!(human_bytes(5 * 1024 * 1024 + 512 * 1024), "5.50M");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_live_process_has_a_positive_resident_set() {
        let memory = snapshot().expect("Linux procfs reports VmRSS");
        assert!(memory.resident_bytes > 0);
        assert_eq!(memory.resident_bytes % 1024, 0);
    }

    #[test]
    fn cgroup_limits_handle_max_and_v1_unlimited_sentinels() {
        assert_eq!(parse_cgroup_limit("max\n"), None);
        assert_eq!(parse_cgroup_limit("1073741824\n"), Some(1_073_741_824));
        assert_eq!(parse_cgroup_limit("9223372036854771712\n"), None);
        assert_eq!(parse_cgroup_limit("nonsense"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_cgroup_memory_is_self_consistent() {
        if let Some(memory) = cgroup_snapshot() {
            assert!(memory.current_bytes > 0);
            assert!(memory.limit_bytes.is_none_or(|limit| limit > 0));
        }
    }
}
