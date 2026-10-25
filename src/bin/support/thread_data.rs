//! Separate embedded mode measurements, never mixed with native competitor rows.
use super::*;

pub(super) fn summarize(root: &Path, sha: &str) -> Result<Option<String>> {
    let directory = root.join("thread-modes");
    if !directory.exists() {
        let required = PROFILES.iter().any(|(name, _)| {
            read(&root.join(name).join("environment.txt"), 128 * 1024).is_ok_and(|text| {
                text.lines()
                    .any(|line| line == "Thread-mode measurements: required")
            })
        });
        ensure(!required, "required thread-mode evidence is missing")?;
        return Ok(None);
    } // Immutable older cohorts predate S11.
    let environment = read(&directory.join("environment.txt"), 128 * 1024)?;
    ensure(
        metadata(&environment, "Source revision:")? == sha
            && environment.contains("Working-tree status (empty means clean):\n\n"),
        "thread-mode source is stale or dirty",
    )?;
    let policy_required = environment
        .lines()
        .any(|line| line == "Thread-mode polling policy: required");
    let run_marker = environment
        .lines()
        .find(|line| line.starts_with("Thread-mode run budget:"));
    let run_version = match run_marker {
        None => 0,
        Some("Thread-mode run budget: cooperative-v1 (required)") => 1,
        Some("Thread-mode run budget: cooperative-v2 (required)") => 2,
        Some(_) => return Err("unsupported thread-mode run policy".into()),
    };
    let run_required = run_version != 0;
    let mut run_policies = Vec::new();
    let mut policies = Vec::new();
    let mut signatures = Vec::new();
    let mut groups = Vec::new();
    for workers in [1, 4] {
        let value = json::parse(&read(
            &directory.join(format!("threads-{workers}.json")),
            2 * 1024 * 1024,
        )?)?;
        let configuration = value.get("server_configuration")?;
        ensure(
            integer(&value, "schema_version")? == 1
                && text(&value, "mode")? == "embedded"
                && text(configuration, "mode")? == "fixed-workers"
                && integer(configuration, "workers")? == workers,
            "thread-mode identity mismatch",
        )?;
        let policy = configuration.get("active_poll_us");
        let active_poll = match policy {
            Ok(value) => value.integer()?,
            Err(_) if !policy_required => 0, // Historical cohorts predate bounded active polling.
            Err(_) => return Err("required thread-mode polling policy is missing".into()),
        };
        ensure(
            [0, 50].contains(&active_poll),
            "unsupported thread-mode polling policy",
        )?;
        ensure(
            !policy_required || active_poll == rillcache::server::FIXED_WORKER_ACTIVE_POLL_US,
            "required thread-mode polling policy differs from runtime",
        )?;
        policies.push(active_poll);
        let run = match configuration.get("run_quantum_us") {
            Ok(value) => value.integer()?,
            Err(_) if !run_required => 0,
            Err(_) => return Err("required thread-mode run budget is missing".into()),
        };
        // Versioned evidence describes its source policy, not a future reader's
        // possibly retuned runtime constant. Poll-only cohorts remain unchanged.
        let expected_run = if workers > 1 { 100 } else { 0 }; // cooperative-v1/v2
        ensure(
            if run_required {
                run == expected_run
            } else {
                run == 0
            },
            "thread-mode run budget differs from runtime",
        )?;
        let expected_version = if workers > 1 { run_version } else { 0 };
        let recorded_version = match configuration.get("run_policy_version") {
            Ok(value) => value.integer()?,
            Err(_) if run_version < 2 => expected_version, // v1 did not serialize a version.
            Err(_) => return Err("required thread-mode run version is missing".into()),
        };
        ensure(
            recorded_version == expected_version,
            "thread-mode run version mismatch",
        )?;
        run_policies.push(run);
        let records = value.get("results")?.array()?;
        ensure(
            records.len() == 15,
            "thread modes need five workloads and three repetitions",
        )?;
        let mut seen = BTreeSet::new();
        let mut samples: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
        for record in records {
            let workload = text(record, "workload")?;
            let run = integer(record, "run")?;
            ensure(
                WORKLOADS.contains(&workload)
                    && (1..=3).contains(&run)
                    && seen.insert((workload, run)),
                "duplicate or missing thread-mode workload/run",
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
                    && (1..=65536).contains(&signature.pipeline)
                    && (1..=1_000_000).contains(&signature.keys)
                    && signature.bytes <= 1048576
                    && signature.warmup <= 1_000_000,
                "invalid mode benchmark parameters",
            )?;
            let batches: u64 = (0..signature.clients)
                .map(|client| {
                    (signature.requests / signature.clients
                        + u64::from(client < signature.requests % signature.clients))
                    .div_ceil(signature.pipeline)
                })
                .sum();
            ensure(
                integer(record, "batches")? == batches,
                "thread-mode batch count mismatch",
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
                "thread-mode timer/rate mismatch",
            )?;
            let latencies = [
                "batch_p50_us",
                "batch_p95_us",
                "batch_p99_us",
                "batch_max_us",
            ]
            .map(|key| number(record, key))
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
            ensure(
                latencies.windows(2).all(|pair| pair[0] <= pair[1]),
                "thread-mode percentiles unordered",
            )?;
            ensure(
                matches!(record.get("server_profile")?, Json::Null),
                "unexpected embedded process accounting",
            )?;
            signatures.push(signature);
            samples
                .entry(workload.into())
                .or_default()
                .push((rate, latencies[2]));
        }
        groups.push(samples);
    }
    ensure(
        signatures
            .iter()
            .all(|signature| signature == &signatures[0]),
        "cross-mode workload parameters differ",
    )?;
    ensure(
        policies.iter().all(|policy| *policy == policies[0]),
        "cross-mode polling policies differ",
    )?;
    let policy_note = if policies[0] == 0 {
        "No post-progress active polling was recorded.".to_owned()
    } else {
        format!(
            "They actively poll for at most {} µs after progress before returning to idle parking; this trades active CPU work for fewer premature sleeps, not epoll readiness or free CPU.",
            policies[0]
        )
    };
    let resume_note = if run_version == 2 {
        "The v2 interval restarts after a yield/park returns, excluding that explicit wait; involuntary descheduling can still consume wall time."
    } else {
        "The initial v1 interval restarts at the yield request, so descheduling can consume its next budget."
    };
    let run_note = if run_policies[1] == 0 {
        "No separate multi-lane cooperative run budget was recorded.".to_owned()
    } else {
        format!(
            "Multiple lanes also request a cooperative yield after {} µs of continuous active passes when more than one client is admitted. Progress does not rearm that separate budget; single lanes/lone clients do not periodically yield. This is checked between bounded passes, not a hard deadline or an OS fairness guarantee. {resume_note}",
            run_policies[1]
        )
    };
    let signature = &signatures[0];
    let mut out = format!(
        "\n### Fixed-worker mode measurements\n\n30/30 separate embedded records validated. {} clients; {} commands/workload/run; pipeline {}; three repetitions. These share the client/server host and inherited CPU affinity; they are **not native competitor comparisons or equal-per-core scaling evidence**. No process CPU/RSS accounting was collected for this group. Fixed lanes have a 1 ms idle polling wait. {policy_note} {run_note}\n\n| Workload | 1 worker median cmd/s | 1 worker min–max | 4 workers median cmd/s | 4 workers min–max | 1 worker batch p99 µs | 4 workers batch p99 µs |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n",
        signature.clients, signature.requests, signature.pipeline
    );
    for workload in WORKLOADS {
        let one = &groups[0][workload];
        let four = &groups[1][workload];
        let rates = |values: &[(f64, f64)]| {
            (
                median(values.iter().map(|v| v.0)),
                values.iter().map(|v| v.0).fold(f64::INFINITY, f64::min),
                values.iter().map(|v| v.0).fold(0.0, f64::max),
            )
        };
        let a = rates(one);
        let b = rates(four);
        writeln!(
            out,
            "| {workload} | {:.0} | {:.0}–{:.0} | {:.0} | {:.0}–{:.0} | {:.1} | {:.1} |",
            a.0,
            a.1,
            a.2,
            b.0,
            b.1,
            b.2,
            median(one.iter().map(|v| v.1)),
            median(four.iter().map(|v| v.1))
        )
        .expect("String write");
    }
    Ok(Some(out))
}
