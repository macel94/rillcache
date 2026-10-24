//! Validate a comparison cohort before producing any comparative Markdown.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use super::json::{self, Json, Result};

#[path = "thread_data.rs"]
mod thread_data;

pub(super) const PROFILES: [(&str, u64); 4] = [
    ("pipeline-1", 1),
    ("pipeline-16", 16),
    ("single-core-pipeline-1", 1),
    ("single-core-pipeline-16", 16),
];
const SERVERS: [&str; 3] = ["rillcache", "redis", "valkey"];
const WORKLOADS: [&str; 5] = ["ping", "set", "get", "set-nx-existing", "set-xx-existing"];

pub(super) fn read(path: &Path, maximum: u64) -> Result<String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "file unavailable")?
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "file unreadable")?;
    if bytes.len() as u64 > maximum {
        return Err("file exceeds size limit".into());
    }
    String::from_utf8(bytes).map_err(|_| "file is not UTF-8".into())
}

pub(super) fn escaped(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            '&' => "&amp;".chars().collect::<Vec<_>>(),
            '<' => "&lt;".chars().collect(),
            '>' => "&gt;".chars().collect(),
            '[' | ']' | '|' | '`' | '*' | '_' | '#' | '!' | '\\' => vec!['\\', c],
            c if c.is_control() && c != '\n' && c != '\t' => vec![],
            c => vec![c],
        })
        .collect()
}

fn integer(value: &Json, key: &str) -> Result<u64> {
    value.get(key)?.integer()
}
fn number(value: &Json, key: &str) -> Result<f64> {
    let number = value.get(key)?.number()?;
    if number < 0.0 {
        return Err(format!("negative {key}"));
    }
    Ok(number)
}
fn text<'a>(value: &'a Json, key: &str) -> Result<&'a str> {
    value.get(key)?.text()
}
fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Signature {
    requests: u64,
    clients: u64,
    pipeline: u64,
    keys: u64,
    bytes: u64,
    warmup: u64,
}

#[derive(Clone, Copy)]
struct Accounting {
    cpu: f64,
    cpu_seconds: f64,
    system_share: f64,
    rss: f64,
    peak: f64,
    voluntary: f64,
    involuntary: f64,
    stable: bool,
    seconds: f64,
}

fn identity(value: &Json) -> Result<(u64, u64, u64)> {
    ensure(
        text(value, "source")? == "linux-procfs",
        "unknown accounting source",
    )?;
    let identity = (
        integer(value, "pid")?,
        integer(value, "process_start_ticks")?,
        integer(value, "clock_ticks_per_second")?,
    );
    ensure(
        identity.0 > 0 && identity.2 > 0,
        "invalid process identity or clock frequency",
    )?;
    Ok(identity)
}

fn accounting(value: &Json) -> Result<Accounting> {
    let (_, _, hz) = identity(value)?;
    let seconds = number(value, "observation_seconds")?;
    ensure(seconds >= 1e-9, "sub-nanosecond or empty accounting window")?;
    let user = number(value, "user_seconds")?;
    let system = number(value, "system_seconds")?;
    let total = user + system;
    let cpu = number(value, "cpu_percent_one_logical_cpu")?;
    ensure(
        total.is_finite()
            && near(user, integer(value, "user_ticks")? as f64 / hz as f64, 1e-8)
            && near(
                system,
                integer(value, "system_ticks")? as f64 / hz as f64,
                1e-8,
            )
            && near(cpu, 100.0 * total / seconds, 0.002),
        "inconsistent CPU counters",
    )?;
    let matched = integer(value, "matched_threads")?;
    let before = integer(value, "sampled_threads_before")?;
    let after = integer(value, "sampled_threads_after")?;
    let stable = value.get("thread_set_unchanged")?.boolean()?;
    ensure(
        matched > 0 && matched <= before && matched <= after,
        "invalid matched-thread coverage",
    )?;
    ensure(
        !stable
            || (matched == before
                && matched == after
                && before == integer(value, "threads_before")?
                && after == integer(value, "threads_after")?),
        "inconsistent stable-thread coverage",
    )?;
    ensure(
        text(value, "context_switches_scope")? == "matched_thread_lifetimes",
        "unknown context-switch scope",
    )?;
    let rss = integer(value, "rss_bytes_after")? as f64 / 1048576.0;
    ensure(
        rss > 0.0 && integer(value, "rss_bytes_before")? > 0,
        "missing RSS endpoints",
    )?;
    Ok(Accounting {
        cpu,
        cpu_seconds: total,
        system_share: if total == 0.0 {
            0.0
        } else {
            100.0 * system / total
        },
        rss,
        peak: integer(value, "lifetime_peak_rss_bytes")? as f64 / 1048576.0,
        voluntary: integer(value, "voluntary_context_switches")? as f64,
        involuntary: integer(value, "involuntary_context_switches")? as f64,
        stable,
        seconds,
    })
}

struct Sample {
    workload: String,
    rate: f64,
    p99: f64,
    signature: Signature,
    accounting: Accounting,
}
struct Row {
    server: &'static str,
    workload: &'static str,
    rate: f64,
    minimum: f64,
    maximum: f64,
    p99: f64,
    cpu: f64,
    cpu_us: f64,
    system_share: f64,
    rss: f64,
    voluntary: f64,
    involuntary: f64,
    stable: bool,
}
struct Profile {
    name: &'static str,
    signature: Signature,
    rows: Vec<Row>,
    idle: Vec<(&'static str, Accounting)>,
    provenance: Provenance,
    server_cpus: String,
    client_cpus: String,
}
#[derive(PartialEq, Eq)]
struct Provenance {
    binaries: BTreeMap<String, String>,
    cpu: String,
    compiler: String,
    redis: String,
    valkey: String,
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut values: Vec<_> = values.collect();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn load_server(
    root: &Path,
    name: &'static str,
    pipeline: u64,
) -> Result<(Signature, Vec<Row>, Accounting)> {
    let value = json::parse(&read(&root.join(format!("{name}.json")), 2 * 1024 * 1024)?)?;
    ensure(
        integer(&value, "schema_version")? == 1
            && text(&value, "label")? == name
            && text(&value, "mode")? == "external"
            && text(&value, "os")? == "linux"
            && integer(&value, "logical_cpus")? > 0,
        "unexpected server, mode or schema",
    )?;
    let idle = value.get("idle_profile")?;
    let process = identity(idle)?;
    let idle = accounting(idle)?;
    let results = value.get("results")?.array()?;
    ensure(
        results.len() == 15,
        "expected five workloads and three repetitions",
    )?;
    let mut seen = BTreeSet::new();
    let mut samples = Vec::new();
    for record in results {
        let workload = text(record, "workload")?;
        let run = integer(record, "run")?;
        ensure(
            WORKLOADS.contains(&workload)
                && (1..=3).contains(&run)
                && seen.insert((workload.to_owned(), run)),
            "duplicate or unknown workload/repetition",
        )?;
        let signature = Signature {
            requests: integer(record, "requests")?,
            clients: integer(record, "clients")?,
            pipeline: integer(record, "pipeline")?,
            keys: integer(record, "keyspace")?,
            bytes: integer(record, "value_size")?,
            warmup: integer(record, "warmup_per_client")?,
        };
        ensure(
            (1..=10_000_000).contains(&signature.requests)
                && (1..=128).contains(&signature.clients)
                && signature.clients <= signature.requests
                && signature.pipeline == pipeline
                && (1..=1_000_000).contains(&signature.keys)
                && signature.bytes <= 1048576
                && signature.warmup <= 1_000_000,
            "invalid workload resource parameters",
        )?;
        let batches: u64 = (0..signature.clients)
            .map(|client| {
                (signature.requests / signature.clients
                    + u64::from(client < signature.requests % signature.clients))
                .div_ceil(pipeline)
            })
            .sum();
        ensure(
            integer(record, "batches")? == batches,
            "incorrect batch count",
        )?;
        let elapsed = number(record, "elapsed_seconds")?;
        let rate = number(record, "ops_per_second")?;
        ensure(
            elapsed >= 1e-9
                && rate > 0.0
                && near(
                    rate,
                    signature.requests as f64 / elapsed,
                    rate * 1e-6 + 0.02,
                ),
            "inconsistent throughput/timer",
        )?;
        let percentiles = [
            "batch_p50_us",
            "batch_p95_us",
            "batch_p99_us",
            "batch_max_us",
        ]
        .map(|key| number(record, key))
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
        ensure(
            percentiles.windows(2).all(|w| w[0] <= w[1]),
            "unordered latency percentiles",
        )?;
        let profile = record.get("server_profile")?;
        ensure(
            identity(profile)? == process,
            "process identity changed within server results",
        )?;
        let accounting = accounting(profile)?;
        ensure(
            accounting.seconds >= elapsed,
            "CPU observation does not bracket request timer",
        )?;
        samples.push(Sample {
            workload: workload.into(),
            rate,
            p99: percentiles[2],
            signature,
            accounting,
        });
    }
    let signature = samples[0].signature.clone();
    ensure(
        samples.iter().all(|s| s.signature == signature),
        "mismatched workload parameters",
    )?;
    let rows = WORKLOADS
        .into_iter()
        .map(|workload| {
            let selected: Vec<_> = samples.iter().filter(|s| s.workload == workload).collect();
            Row {
                server: name,
                workload,
                rate: median(selected.iter().map(|s| s.rate)),
                minimum: selected
                    .iter()
                    .map(|s| s.rate)
                    .fold(f64::INFINITY, f64::min),
                maximum: selected.iter().map(|s| s.rate).fold(0.0, f64::max),
                p99: median(selected.iter().map(|s| s.p99)),
                cpu: median(selected.iter().map(|s| s.accounting.cpu)),
                cpu_us: median(
                    selected
                        .iter()
                        .map(|s| s.accounting.cpu_seconds * 1e6 / signature.requests as f64),
                ),
                system_share: median(selected.iter().map(|s| s.accounting.system_share)),
                rss: median(selected.iter().map(|s| s.accounting.rss)),
                voluntary: median(
                    selected
                        .iter()
                        .map(|s| s.accounting.voluntary / signature.requests as f64),
                ),
                involuntary: median(
                    selected
                        .iter()
                        .map(|s| s.accounting.involuntary / signature.requests as f64),
                ),
                stable: selected.iter().all(|s| s.accounting.stable),
            }
        })
        .collect();
    Ok((signature, rows, idle))
}

fn metadata<'a>(text: &'a str, prefix: &str) -> Result<&'a str> {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .ok_or_else(|| format!("missing metadata {prefix}"))
}

fn binaries(text: &str) -> Result<BTreeMap<String, String>> {
    let mut found = BTreeMap::new();
    for line in text.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() != 2
            || parts[0].len() != 64
            || !parts[0].bytes().all(|b| b.is_ascii_hexdigit())
        {
            continue;
        }
        for name in ["rillcache", "redis-server", "valkey-server"] {
            if parts[1].ends_with(&format!("/{name}")) {
                ensure(
                    found
                        .insert(name.into(), parts[0].to_ascii_lowercase())
                        .is_none(),
                    "duplicate executable digest",
                )?;
            }
        }
    }
    Ok(found)
}

fn cpus(value: &str) -> Result<BTreeSet<u64>> {
    let mut found = BTreeSet::new();
    for part in value.split(',') {
        let (first, last) = part.split_once('-').unwrap_or((part, part));
        let first: u64 = first.parse().map_err(|_| "invalid CPU list")?;
        let last: u64 = last.parse().map_err(|_| "invalid CPU list")?;
        ensure(last >= first && last - first < 4096, "invalid CPU range")?;
        found.extend(first..=last);
    }
    ensure(
        !found.is_empty() && found.len() <= 4096,
        "empty or oversized CPU list",
    )?;
    Ok(found)
}

fn load_profile(
    root: &Path,
    name: &'static str,
    pipeline: u64,
    sha: &str,
    references: &BTreeMap<String, String>,
) -> Result<Profile> {
    let directory = root.join(name);
    let environment = read(&directory.join("environment.txt"), 128 * 1024)?;
    ensure(
        metadata(&environment, "Source revision:")? == sha,
        "source SHA mismatch",
    )?;
    ensure(
        environment.contains("Working-tree status (empty means clean):\n\n"),
        "dirty or missing source status",
    )?;
    ensure(
        metadata(&environment, "Profiling:")? == "1"
            && metadata(&environment, "Server lifecycle:")?
                == "one live comparison server at a time",
        "unprofiled or nonisolated cohort",
    )?;
    let server_cpus = metadata(&environment, "Server CPU request:")?.to_owned();
    let client_cpus = metadata(&environment, "Client CPU request:")?.to_owned();
    if name.starts_with("single-core") {
        let servers = cpus(&server_cpus)?;
        let clients = cpus(&client_cpus)?;
        ensure(
            servers.len() == 1 && servers.is_disjoint(&clients),
            "server/client affinity budgets overlap or are not single-core",
        )?;
    } else {
        ensure(
            server_cpus == "inherited" && client_cpus == "inherited",
            "default-threading profile has unexpected affinity",
        )?;
    }
    let digests = binaries(&environment)?;
    ensure(
        digests.len() == 3
            && references.len() == 2
            && references
                .iter()
                .all(|(name, hash)| digests.get(name) == Some(hash)),
        "reference binary provenance mismatch",
    )?;
    let provenance = Provenance {
        binaries: digests,
        cpu: metadata(&environment, "Model name:")?.into(),
        compiler: metadata(&environment, "rustc ")?.into(),
        redis: metadata(&environment, "Redis server v=")?.into(),
        valkey: metadata(&environment, "Valkey server v=")?.into(),
    };
    let mut rows = Vec::new();
    let mut idle = Vec::new();
    let mut signature = None;
    for server in SERVERS {
        let (parameters, values, accounting) = load_server(&directory, server, pipeline)?;
        if let Some(previous) = &signature {
            ensure(previous == &parameters, "cross-server parameter mismatch")?;
        }
        signature = Some(parameters);
        rows.extend(values);
        idle.push((server, accounting));
    }
    rows.sort_by_key(|row| {
        (
            WORKLOADS
                .iter()
                .position(|name| *name == row.workload)
                .expect("validated workload"),
            SERVERS
                .iter()
                .position(|name| *name == row.server)
                .expect("validated server"),
        )
    });
    Ok(Profile {
        name,
        signature: signature.expect("three servers"),
        rows,
        idle,
        provenance,
        server_cpus,
        client_cpus,
    })
}

pub(super) struct Summary {
    pub(super) markdown: String,
    pub(super) complete: bool,
    pub(super) records: usize,
}

pub(super) fn summarize(root: &Path, sha: &str) -> Summary {
    let mut issues = Vec::new();
    let mut profiles = Vec::new();
    let references =
        read(&root.join("reference-binaries.sha256"), 4096).and_then(|text| binaries(&text));
    for (name, pipeline) in PROFILES {
        let result = references
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|refs| load_profile(root, name, pipeline, sha, refs));
        match result {
            Ok(profile) => profiles.push(profile),
            Err(error) => issues.push(format!("{name}: {error}")),
        }
    }
    if let Some(first) = profiles.first()
        && profiles.iter().any(|p| p.provenance != first.provenance)
    {
        issues.push("Cross-profile executable, compiler, hardware or version identity mismatch; cohort rejected.".into());
        profiles.clear();
    }
    let thread_modes = match thread_data::summarize(root, sha) {
        Ok(value) => value,
        Err(error) => {
            issues.push(format!("thread modes: {error}"));
            None
        }
    };
    let records = profiles.len() * 45;
    let complete = issues.is_empty() && profiles.len() == 4;
    let mut out = format!(
        "## Measurements\n\n**Data quality: {}.** {records}/180 records validated across {}/4 complete profiles.\n\n",
        if complete {
            "complete"
        } else {
            "INCOMPLETE / INVALID"
        },
        profiles.len()
    );
    for issue in issues {
        writeln!(out, "- {}", escaped(&issue)).expect("String write");
    }
    if let Some(first) = profiles.first() {
        writeln!(
            out,
            "\nHardware: {}. Rust {}.\n\nReferences: Redis {}; Valkey {}.\n",
            escaped(&first.provenance.cpu),
            escaped(&first.provenance.compiler),
            escaped(&first.provenance.redis),
            escaped(&first.provenance.valkey)
        )
        .expect("String write");
        out.push_str("### SET at a glance\n\nMedian commands/s and median of the three runs' **whole-batch p99** values (µs). Positive rate delta means Rillcache is faster in this workload only.\n\n| Profile | Rillcache cmd/s | Redis | Valkey | Rillcache vs Redis | Rillcache p99 µs | Redis p99 | Valkey p99 |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
        for profile in &profiles {
            let rows: Vec<_> = profile
                .rows
                .iter()
                .filter(|r| r.workload == "set")
                .collect();
            writeln!(
                out,
                "| {} | {:.0} | {:.0} | {:.0} | {:+.1}% | {:.1} | {:.1} | {:.1} |",
                profile.name,
                rows[0].rate,
                rows[1].rate,
                rows[2].rate,
                (rows[0].rate / rows[1].rate - 1.0) * 100.0,
                rows[0].p99,
                rows[1].p99,
                rows[2].p99
            )
            .expect("String write");
        }
    }
    out.push_str("\nCPU is relative to **one logical CPU**, so parallel servers can exceed 100%. Do not blend default-threading and single-core results or different pipelines. Shared VMs and unequal feature sets do not establish a general performance win or a cross-run regression.\n");
    for profile in profiles {
        let s = &profile.signature;
        writeln!(out, "\n<details>\n<summary>{}: all five workloads, CPU, memory and scheduling</summary>\n\nRequests/workload/run: {}; clients: {}; pipeline: {}; cyclic keys: {}; value bytes: {}; warmup/client: {}; repetitions: 3. Server CPUs: {}; client CPUs: {}.\n", profile.name, s.requests, s.clients, s.pipeline, s.keys, s.bytes, s.warmup, escaped(&profile.server_cpus), escaped(&profile.client_cpus)).expect("String write");
        out.push_str("| Workload | Server | Median cmd/s | Min–max cmd/s | Batch p99 µs | CPU % | CPU µs/cmd | System % of CPU | RSS MiB | Voluntary switches/cmd | Involuntary/cmd | Thread endpoints |\n| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n");
        for row in profile.rows {
            writeln!(out, "| {} | {} | {:.0} | {:.0}–{:.0} | {:.1} | {:.1} | {:.2} | {:.1} | {:.2} | {:.6} | {:.6} | {} |", row.workload, row.server, row.rate, row.minimum, row.maximum, row.p99, row.cpu, row.cpu_us, row.system_share, row.rss, row.voluntary, row.involuntary, if row.stable { "unchanged" } else { "PARTIAL" }).expect("String write");
        }
        out.push_str("\n| Idle server | Observation s | CPU % | RSS MiB | Lifetime peak MiB | Voluntary switches/s | Thread endpoints |\n| --- | ---: | ---: | ---: | ---: | ---: | --- |\n");
        for (name, idle) in profile.idle {
            writeln!(
                out,
                "| {name} | {:.3} | {:.3} | {:.2} | {:.2} | {:.1} | {} |",
                idle.seconds,
                idle.cpu,
                idle.rss,
                idle.peak,
                idle.voluntary / idle.seconds,
                if idle.stable { "unchanged" } else { "PARTIAL" }
            )
            .expect("String write");
        }
        out.push_str("\n</details>\n");
    }
    if let Some(thread_modes) = thread_modes {
        out.push_str(&thread_modes);
    }
    out.push_str("\n### Interpretation limits\n\n- Batch p99 is **not per-request p99**; never divide it by the pipeline length. Percentiles are not pooled across runs.\n- CPU counters are tick-quantized; zero observed idle ticks do not prove zero CPU cost. CPU observation windows bracket, but are not identical to, request timers.\n- RSS is an approximate endpoint sample, not allocator accounting or a workload peak; VmHWM includes startup, warmup and previous workloads.\n- Context switches cover matched thread lifetimes only. Even unchanged endpoints cannot exclude threads born and destroyed between snapshots. PARTIAL coverage is not a process total.\n- NX-existing measures rejected writes; XX-existing measures admitted overwrites. This is a small Redis-compatible subset, not equivalent functionality.\n");
    Summary {
        markdown: out,
        complete,
        records,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archived_real_cohort_produces_correct_medians_without_network() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/ci/10d56ec");
        let report = summarize(&root, "10d56eca165f14a2b5fb043e36d2c33cc52ed762");
        assert!(report.complete, "{}", report.markdown);
        assert_eq!(report.records, 180);
        assert!(report.markdown.contains(
            "| single-core-pipeline-1 | 48952 | 58672 | 57526 | -16.6% | 326.5 | 188.1 | 192.2 |"
        ));
        assert!(
            report
                .markdown
                .contains("| single-core-pipeline-16 | 673558 | 597827 | 538714 | +12.7%")
        );
        assert_eq!(report.markdown.matches("<details>").count(), 4);
        assert_eq!(report.markdown.matches("| set-nx-existing |").count(), 12);
    }

    #[test]
    fn missing_and_wrong_source_evidence_cannot_become_performance_results() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/ci/10d56ec");
        for report in [
            summarize(&root, &"0".repeat(40)),
            summarize(&root.join("absent"), &"0".repeat(40)),
        ] {
            assert!(!report.complete);
            assert_eq!(report.records, 0);
            assert!(!report.markdown.contains("### SET at a glance"));
        }
    }

    #[test]
    fn affinity_lists_and_markup_are_bounded_and_unambiguous() {
        assert_eq!(cpus("1,2-3").unwrap(), BTreeSet::from([1, 2, 3]));
        for value in ["", "inherited", "-1", "4-2", "0-1000000", "1,,2"] {
            assert!(cpus(value).is_err());
        }
        assert_eq!(
            escaped("<img> [link](url)|`"),
            "&lt;img&gt; \\[link\\](url)\\|\\`"
        );
    }
}
