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

pub(crate) fn snapshot() -> Option<ProcessMemory> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let resident_kib = parse_vm_rss_kib(&status)?;
    Some(ProcessMemory {
        resident_bytes: resident_kib.checked_mul(1024)?,
    })
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
}
