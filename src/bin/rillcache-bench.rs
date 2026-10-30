#![forbid(unsafe_code)]

//! The same validated TCP workload generator for rillcache, Redis and Valkey.

use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::ExitCode;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rillcache::protocol::{encode_bulk, encode_request};
use rillcache::{Config, Server};

#[path = "support/profile.rs"]
mod profile;

const HELP: &str = "Usage: rillcache-bench [OPTIONS]\n\nWithout --addr, starts and shuts down a local embedded rillcache server.\nJSON results go to stdout; progress and errors go to stderr.\n\n  --addr IP:PORT       Benchmark an existing server instead\n  --label TEXT         Identify the server/build in JSON\n  --server-threads N   Embedded fixed worker count, 1..64 (not with --addr)\n  --server-pid PID     Opt-in Linux accounting for this loopback server process\n  --profile-idle-ms N  Sample idle server before workloads (0..10000, default 0)\n  --requests N         Total measured commands per workload (default 100000)\n  --clients N          Parallel connections/threads (default 8, maximum 128)\n  --pipeline N         Commands per batch (default 16)\n  --keyspace N         Deterministically rotated keys (default 1024)\n  --value-size N       Value bytes, including zero (default 64)\n  --warmup N           Untimed commands per client (default 1000)\n  --runs N             Repetitions of each workload (default 3)\n  --workload NAME      ping, set, get, set-nx-existing, set-xx-existing, all (default)\n  --help              Show this help\n\nWrites only rillcache:bench:* keys. Use a disposable server, not production.\nLatency percentiles describe whole pipeline batches, NOT individual requests.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Workload {
    Ping,
    Set,
    Get,
    SetNxExisting,
    SetXxExisting,
}

impl Workload {
    const ALL: [Self; 5] = [
        Self::Ping,
        Self::Set,
        Self::Get,
        Self::SetNxExisting,
        Self::SetXxExisting,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::Set => "set",
            Self::Get => "get",
            Self::SetNxExisting => "set-nx-existing",
            Self::SetXxExisting => "set-xx-existing",
        }
    }
}

#[derive(Clone, Debug)]
struct Options {
    addr: Option<SocketAddr>,
    label: Option<String>,
    server_pid: Option<u32>,
    server_threads: Option<usize>,
    profile_idle_ms: usize,
    requests: usize,
    clients: usize,
    pipeline: usize,
    keyspace: usize,
    value_size: usize,
    warmup: usize,
    runs: usize,
    workloads: Vec<Workload>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            addr: None,
            label: None,
            server_pid: None,
            server_threads: None,
            profile_idle_ms: 0,
            requests: 100_000,
            clients: 8,
            pipeline: 16,
            keyspace: 1024,
            value_size: 64,
            warmup: 1000,
            runs: 3,
            workloads: Workload::ALL.to_vec(),
        }
    }
}

fn options(arguments: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options::default();
    let mut arguments = arguments.into_iter();
    while let Some(option) = arguments.next() {
        if option == "--help" || option == "-h" {
            return Ok(None);
        }
        if ![
            "--addr",
            "--label",
            "--server-pid",
            "--server-threads",
            "--profile-idle-ms",
            "--requests",
            "--clients",
            "--pipeline",
            "--keyspace",
            "--value-size",
            "--warmup",
            "--runs",
            "--workload",
        ]
        .contains(&option.as_str())
        {
            return Err(format!("unknown option: {option}"));
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--addr" => {
                options.addr = Some(
                    value
                        .parse()
                        .map_err(|_| "--addr requires a numeric IP:port")?,
                )
            }
            "--label" => options.label = Some(value),
            "--server-threads" => {
                let count = value
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=64).contains(n))
                    .ok_or("--server-threads must be in 1..=64")?;
                options.server_threads = Some(count);
            }
            "--server-pid" => {
                options.server_pid = Some(
                    value
                        .parse::<u32>()
                        .ok()
                        .filter(|&pid| pid > 0)
                        .ok_or("--server-pid requires a positive 32-bit PID")?,
                );
            }
            "--workload" => options.workloads = match value.as_str() {
                "ping" => vec![Workload::Ping],
                "set" => vec![Workload::Set],
                "get" => vec![Workload::Get],
                "set-nx-existing" => vec![Workload::SetNxExisting],
                "set-xx-existing" => vec![Workload::SetXxExisting],
                "all" => Workload::ALL.to_vec(),
                _ => return Err(
                    "--workload must be ping, set, get, set-nx-existing, set-xx-existing or all"
                        .into(),
                ),
            },
            _ => {
                let number = value
                    .parse::<usize>()
                    .map_err(|_| format!("{option} requires a nonnegative integer"))?;
                match option.as_str() {
                    "--requests" => options.requests = number,
                    "--clients" => options.clients = number,
                    "--pipeline" => options.pipeline = number,
                    "--keyspace" => options.keyspace = number,
                    "--value-size" => options.value_size = number,
                    "--warmup" => options.warmup = number,
                    "--profile-idle-ms" => options.profile_idle_ms = number,
                    "--runs" => options.runs = number,
                    _ => unreachable!(),
                }
            }
        }
    }
    for (name, number, maximum) in [
        ("requests", options.requests, 10_000_000),
        ("clients", options.clients, 128),
        ("pipeline", options.pipeline, 65_536),
        ("keyspace", options.keyspace, 1_000_000),
        ("runs", options.runs, 100),
    ] {
        if number == 0 || number > maximum {
            return Err(format!("--{name} must be in 1..={maximum}"));
        }
    }
    if options.addr.is_some() && options.server_threads.is_some() {
        return Err(
            "--server-threads configures embedded servers only; cannot be used with --addr".into(),
        );
    }
    if options.clients > options.requests {
        return Err("--clients must not exceed --requests".into());
    }
    if options.value_size > 1024 * 1024 || options.warmup > 1_000_000 {
        return Err("--value-size must be <= 1048576 and --warmup <= 1000000".into());
    }
    if options.profile_idle_ms > 10_000 {
        return Err("--profile-idle-ms must be <= 10000".into());
    }
    if options.profile_idle_ms > 0 && options.server_pid.is_none() {
        return Err("--profile-idle-ms requires --server-pid".into());
    }
    if let Some(pid) = options.server_pid {
        if !cfg!(target_os = "linux") {
            return Err("--server-pid requires Linux procfs".into());
        }
        if pid == std::process::id() || !options.addr.is_some_and(|addr| addr.ip().is_loopback()) {
            return Err(
                "--server-pid requires --addr on loopback and a separate server process".into(),
            );
        }
    }
    let template_bytes = options.keyspace.checked_mul(options.value_size + 128);
    let batch_bytes = options.pipeline.checked_mul(options.value_size + 128);
    if template_bytes.is_none_or(|n| n > 64 * 1024 * 1024)
        || batch_bytes.is_none_or(|n| n > 16 * 1024 * 1024)
    {
        return Err("key templates must fit in 64 MiB and a pipeline batch in 16 MiB; reduce keyspace, pipeline or value size".into());
    }
    Ok(Some(options))
}

struct Commands {
    encoded: Vec<Vec<u8>>,
    expected: Vec<u8>,
}

impl Commands {
    fn new(workload: Workload, options: &Options) -> Self {
        let value = vec![b'x'; options.value_size];
        let mut encoded = Vec::new();
        for key in 0..if workload == Workload::Ping {
            1
        } else {
            options.keyspace
        } {
            let key = format!("rillcache:bench:{key:012}");
            let arguments: Vec<&[u8]> = match workload {
                Workload::Ping => vec![b"PING"],
                Workload::Set => vec![b"SET", key.as_bytes(), &value],
                Workload::Get => vec![b"GET", key.as_bytes()],
                Workload::SetNxExisting => vec![b"SET", key.as_bytes(), &value, b"NX"],
                Workload::SetXxExisting => vec![b"SET", key.as_bytes(), &value, b"XX"],
            };
            let mut command = Vec::new();
            encode_request(&arguments, &mut command);
            encoded.push(command);
        }
        let mut expected = Vec::new();
        match workload {
            Workload::Ping => expected.extend_from_slice(b"+PONG\r\n"),
            Workload::Set | Workload::SetXxExisting => expected.extend_from_slice(b"+OK\r\n"),
            Workload::Get => encode_bulk(Some(&value), &mut expected),
            Workload::SetNxExisting => encode_bulk(None, &mut expected),
        }
        Self { encoded, expected }
    }
}

struct Connection {
    stream: TcpStream,
    outgoing: Vec<u8>,
    incoming: Vec<u8>,
}

impl Connection {
    fn connect(addr: SocketAddr) -> io::Result<Self> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let stream = loop {
            match TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
                Ok(stream) => break stream,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionRefused | io::ErrorKind::TimedOut
                    ) && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(error) => return Err(error),
            }
        };
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        Ok(Self {
            stream,
            outgoing: Vec::new(),
            incoming: Vec::new(),
        })
    }

    fn exchange(&mut self, commands: &Commands, first: usize, count: usize) -> io::Result<()> {
        self.outgoing.clear();
        for index in first..first + count {
            self.outgoing
                .extend_from_slice(&commands.encoded[index % commands.encoded.len()]);
        }
        self.stream.write_all(&self.outgoing)?;
        self.incoming.resize(commands.expected.len() * count, 0);
        self.stream.read_exact(&mut self.incoming)?;
        if self
            .incoming
            .chunks_exact(commands.expected.len())
            .any(|reply| reply != commands.expected)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "server returned an incorrect response; benchmark aborted",
            ));
        }
        Ok(())
    }

    fn untimed(
        &mut self,
        commands: &Commands,
        total: usize,
        pipeline: usize,
        first: usize,
    ) -> io::Result<()> {
        let mut done = 0;
        while done < total {
            let count = pipeline.min(total - done);
            self.exchange(commands, first + done, count)?;
            done += count;
        }
        Ok(())
    }
}

struct Samples {
    operations: usize,
    batch_ns: Vec<u64>,
}

struct Record {
    workload: Workload,
    run: usize,
    elapsed_seconds: f64,
    batches: usize,
    p50_us: f64,
    p95_us: f64,
    p99_us: f64,
    max_us: f64,
    profile: Option<profile::Measurement>,
}

fn assigned(total: usize, clients: usize, client: usize) -> usize {
    total / clients + usize::from(client < total % clients)
}

fn percentile(sorted: &[u64], percent: usize) -> f64 {
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)] as f64 / 1000.0
}

fn run_one(
    addr: SocketAddr,
    workload: Workload,
    run: usize,
    options: &Options,
    profiler: Option<&profile::Profiler>,
) -> io::Result<Record> {
    let commands = Arc::new(Commands::new(workload, options));
    if !matches!(workload, Workload::Ping | Workload::Set) {
        let seed = Commands::new(Workload::Set, options);
        let verification = Commands::new(Workload::Get, options);
        let mut connection = Connection::connect(addr)?;
        connection.untimed(&seed, options.keyspace, options.pipeline, 0)?;
        connection.untimed(&verification, options.keyspace, options.pipeline, 0)?;
    }
    // A completed TCP handshake is not proof that the server accepted/spawned
    // its worker. Validate a PING even with --warmup 0, outside all measurement.
    let ping = Commands::new(Workload::Ping, options);
    let mut prepared = Vec::new();
    for client in 0..options.clients {
        let mut connection = Connection::connect(addr)?;
        connection.exchange(&ping, 0, 1)?;
        connection.untimed(&commands, options.warmup, options.pipeline, client)?;
        prepared.push(connection);
    }
    let mut starts: Vec<mpsc::Sender<()>> = Vec::new();
    let mut workers: Vec<thread::JoinHandle<io::Result<(Samples, Connection)>>> = Vec::new();
    for (client, mut connection) in prepared.into_iter().enumerate() {
        let commands = Arc::clone(&commands);
        let requests = assigned(options.requests, options.clients, client);
        let pipeline = options.pipeline;
        let mut samples = Samples {
            operations: 0,
            batch_ns: Vec::with_capacity(requests.div_ceil(pipeline)),
        };
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(format!("bench-{client}"))
            .spawn(move || {
                // Dropping senders on a spawn failure wakes all already-created workers.
                receiver
                    .recv()
                    .map_err(|_| io::Error::other("benchmark start cancelled"))?;
                while samples.operations < requests {
                    let count = pipeline.min(requests - samples.operations);
                    let start = Instant::now();
                    connection.exchange(&commands, client + samples.operations, count)?;
                    samples
                        .batch_ns
                        .push(start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
                    samples.operations += count;
                }
                // Retain the socket until the controller's final snapshot, so
                // server client-thread counters do not disappear before sampling.
                Ok((samples, connection))
            });
        match worker {
            Ok(worker) => {
                starts.push(sender);
                workers.push(worker);
            }
            Err(error) => {
                drop(starts);
                for worker in workers {
                    let _ = worker.join();
                }
                return Err(error);
            }
        }
    }
    let before = match profiler.map(profile::Profiler::snapshot).transpose() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            drop(starts);
            for worker in workers {
                let _ = worker.join();
            }
            return Err(error);
        }
    };
    let started = Instant::now();
    let mut failure = None;
    for sender in starts {
        if sender.send(()).is_err() {
            failure = Some(io::Error::other("benchmark worker failed before start"));
        }
    }
    let mut operations = 0;
    let mut samples = Vec::new();
    let mut connections = Vec::new();
    // Join EVERY worker even when one fails; errors must not leave background load.
    for worker in workers {
        match worker.join() {
            Ok(Ok((mut result, connection))) => {
                operations += result.operations;
                samples.append(&mut result.batch_ns);
                connections.push(connection);
            }
            Ok(Err(error)) => failure = Some(error),
            Err(_) => failure = Some(io::Error::other("benchmark worker panicked")),
        }
    }
    let elapsed_seconds = started.elapsed().as_secs_f64();
    if let Some(error) = failure {
        return Err(error);
    }
    if operations != options.requests || samples.is_empty() || elapsed_seconds <= 0.0 {
        return Err(io::Error::other(
            "benchmark did not complete the exact requested operation count",
        ));
    }
    let profile = match (profiler, before) {
        (Some(profiler), Some(before)) => Some(profiler.measure(before, profiler.snapshot()?)?),
        _ => None,
    };
    drop(connections);
    if matches!(workload, Workload::SetNxExisting | Workload::SetXxExisting) {
        let verification = Commands::new(Workload::Get, options);
        Connection::connect(addr)?.untimed(&verification, options.keyspace, options.pipeline, 0)?;
    }
    samples.sort_unstable();
    Ok(Record {
        workload,
        run,
        elapsed_seconds,
        batches: samples.len(),
        p50_us: percentile(&samples, 50),
        p95_us: percentile(&samples, 95),
        p99_us: percentile(&samples, 99),
        max_us: *samples.last().expect("nonempty samples") as f64 / 1000.0,
        profile,
    })
}

fn quoted(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            c if c <= '\u{1f}' => write!(output, "\\u{:04x}", c as u32).expect("String write"),
            c => output.push(c),
        }
    }
    output.push('"');
    output
}

fn profile_json(value: Option<&profile::Measurement>) -> String {
    value.map_or_else(|| "null".into(), profile::Measurement::json)
}

fn report(
    addr: SocketAddr,
    options: &Options,
    records: &[Record],
    idle: Option<&profile::Measurement>,
    scheduling: Option<rillcache::server::SchedulingPolicy>,
) -> String {
    let mode = if options.addr.is_some() {
        "external"
    } else {
        "embedded"
    };
    let label = options
        .label
        .as_deref()
        .unwrap_or(if options.addr.is_some() {
            "external"
        } else {
            "rillcache"
        });
    let mut output = format!(
        "{{\"schema_version\":1,\"tool_version\":{},\"label\":{},\"mode\":{},\"address\":{},\"os\":{},\"arch\":{},\"logical_cpus\":{},\"idle_profile\":{},\"results\":[",
        quoted(env!("CARGO_PKG_VERSION")),
        quoted(label),
        quoted(mode),
        quoted(&addr.to_string()),
        quoted(std::env::consts::OS),
        quoted(std::env::consts::ARCH),
        thread::available_parallelism().map_or(1, usize::from),
        profile_json(idle)
    );
    // Additive mode provenance; external servers must be described by their launcher.
    let configuration = match (options.addr, options.server_threads) {
        (Some(_), _) => "null".to_owned(),
        (None, Some(count)) => {
            let policy = scheduling.expect("embedded scheduling policy");
            format!(
                "{{\"mode\":\"fixed-workers\",\"workers\":{count},\"active_poll_us\":{},\"run_quantum_us\":{},\"run_policy_version\":{}}}",
                policy.active_poll_us,
                policy.run_quantum_us,
                policy.run_policy_version()
            )
        }
        (None, None) => {
            let policy = scheduling.expect("embedded scheduling policy");
            if policy.readiness_io && !policy.tokio_transport {
                "{\"mode\":\"mio-readiness\",\"readiness_io\":true}".to_owned()
            } else if policy.readiness_io {
                format!(
                    "{{\"mode\":\"tokio-current-thread\",\"workers\":{},\"network_runtime_threads\":{},\"accept_runtime_shared\":true,\"cold_io_service_threads\":1,\"storage_execution\":{},\"compatibility_data_owner_threads\":{}}}",
                    policy.tokio_worker_threads,
                    policy.tokio_worker_threads,
                    quoted(if policy.tokio_compatibility_owner {
                        "compatibility-bridge"
                    } else {
                        "local-owner"
                    }),
                    usize::from(policy.tokio_compatibility_owner)
                )
            } else {
                format!(
                    "{{\"mode\":\"thread-per-client\",\"workers\":null,\"single_reply_yield\":{},\"client_handoff_idle_us\":{},\"client_handoff_policy_version\":{},\"client_handoff_max_epoch_ms\":{},\"client_handoff_lanes\":{}}}",
                    policy.single_reply_yield,
                    policy.client_handoff_idle_us,
                    policy.client_handoff_policy_version(),
                    policy.client_handoff_max_epoch_ms(),
                    policy.client_handoff_lanes
                )
            }
        }
    };
    let insertion = output.len() - "\"results\":[".len();
    output.insert_str(
        insertion,
        &format!("\"server_configuration\":{configuration},"),
    );
    for (index, record) in records.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(output, "{{\"workload\":{},\"run\":{},\"requests\":{},\"clients\":{},\"pipeline\":{},\"keyspace\":{},\"value_size\":{},\"warmup_per_client\":{},\"batches\":{},\"elapsed_seconds\":{:.9},\"ops_per_second\":{:.2},\"batch_p50_us\":{:.3},\"batch_p95_us\":{:.3},\"batch_p99_us\":{:.3},\"batch_max_us\":{:.3},\"server_profile\":{}}}", quoted(record.workload.name()), record.run, options.requests, options.clients, options.pipeline, options.keyspace, options.value_size, options.warmup, record.batches, record.elapsed_seconds, options.requests as f64 / record.elapsed_seconds, record.p50_us, record.p95_us, record.p99_us, record.max_us, profile_json(record.profile.as_ref())).expect("String write");
    }
    output.push_str("]}");
    output
}

fn benchmark(options: &Options) -> io::Result<String> {
    let local = if options.addr.is_none() {
        Some(
            Server::bind(Config {
                bind_addr: "127.0.0.1:0".parse().expect("literal address"),
                worker_threads: options.server_threads,
                ..Config::default()
            })?
            .spawn()?,
        )
    } else {
        None
    };
    let addr = options
        .addr
        .unwrap_or_else(|| local.as_ref().expect("embedded server").local_addr());
    let scheduling = local.as_ref().map(|server| server.scheduling_policy());
    let profiler = options.server_pid.map(profile::Profiler::new).transpose()?;
    let idle = match profiler.as_ref() {
        Some(profiler) if options.profile_idle_ms > 0 => {
            let before = profiler.snapshot()?;
            thread::sleep(Duration::from_millis(options.profile_idle_ms as u64));
            Some(profiler.measure(before, profiler.snapshot()?)?)
        }
        _ => None,
    };
    let mut records = Vec::new();
    for run in 1..=options.runs {
        for &workload in &options.workloads {
            let record = run_one(addr, workload, run, options, profiler.as_ref())?;
            eprintln!(
                "{} run {}: {:.0} ops/s, batch p50 {:.1} us, p99 {:.1} us",
                workload.name(),
                run,
                options.requests as f64 / record.elapsed_seconds,
                record.p50_us,
                record.p99_us
            );
            records.push(record);
        }
    }
    if let Some(local) = local {
        local.shutdown()?;
    }
    Ok(report(addr, options, &records, idle.as_ref(), scheduling))
}

fn main() -> ExitCode {
    match options(std::env::args().skip(1)) {
        Ok(None) => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Some(options)) => match benchmark(&options) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("rillcache-bench: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("rillcache-bench: {error}\nUse --help for usage.");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn validates_zero_overflow_and_resource_limits() {
        for arguments in [
            vec!["--clients", "0"],
            vec!["--server-threads", "0"],
            vec!["--server-threads", "65"],
            vec!["--server-threads", "4", "--addr", "127.0.0.1:6379"],
            vec!["--requests", "0"],
            vec!["--pipeline", "0"],
            vec!["--runs", "0"],
            vec!["--keyspace", "0"],
            vec!["--clients", "129"],
            vec!["--requests", "1"],
            vec!["--value-size", "-1"],
            vec!["--workload", "mixed"],
            vec!["--unknown"],
            vec!["--addr"],
            vec!["--addr", "localhost:6379"],
            vec!["--requests", "99999999999999999999999999999"],
            vec!["--keyspace", "1000000", "--value-size", "1048576"],
            vec!["--pipeline", "65536", "--value-size", "1048576"],
            vec!["--server-pid", "0"],
            vec!["--server-pid", "4294967296"],
            vec!["--server-pid", "-1"],
            vec!["--server-pid", "42"],
            vec!["--server-pid", "42", "--addr", "192.0.2.1:6379"],
            vec!["--profile-idle-ms", "1"],
            vec!["--profile-idle-ms", "10001"],
        ] {
            assert!(
                options(arguments.iter().map(|s| (*s).to_string())).is_err(),
                "{arguments:?}"
            );
        }
        assert!(
            options(
                ["--value-size", "0", "--warmup", "0"]
                    .into_iter()
                    .map(str::to_owned)
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn profile_options_reject_the_client_pid_and_accept_separate_loopback_targets() {
        let own_pid = std::process::id().to_string();
        assert!(
            options(
                ["--server-pid", &own_pid, "--addr", "127.0.0.1:6379"]
                    .into_iter()
                    .map(str::to_owned)
            )
            .is_err()
        );
        let result = options(
            [
                "--server-pid",
                "4294967295",
                "--addr",
                "[::1]:6379",
                "--profile-idle-ms",
                "10000",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        assert_eq!(result.is_ok(), cfg!(target_os = "linux"));
    }

    #[test]
    fn zero_warmup_still_validates_an_untimed_ping_before_the_workload() {
        let options = Options {
            requests: 1,
            clients: 1,
            warmup: 0,
            keyspace: 1,
            ..Options::default()
        };
        let ping = Commands::new(Workload::Ping, &options).encoded.remove(0);
        let set = Commands::new(Workload::Set, &options).encoded.remove(0);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            for (expected, reply) in [
                (ping, b"+PONG\r\n".as_slice()),
                (set, b"+OK\r\n".as_slice()),
            ] {
                let mut actual = vec![0; expected.len()];
                stream.read_exact(&mut actual).unwrap();
                assert_eq!(actual, expected);
                stream.write_all(reply).unwrap();
            }
            assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        });
        let record = run_one(addr, Workload::Set, 1, &options, None).unwrap();
        assert_eq!(record.batches, 1);
        assert!(record.profile.is_none());
        server.join().unwrap();
    }

    #[test]
    fn uneven_requests_are_never_dropped() {
        for total in 1..100 {
            for clients in 1..=total.min(16) {
                let counts: Vec<_> = (0..clients)
                    .map(|client| assigned(total, clients, client))
                    .collect();
                assert_eq!(counts.iter().sum::<usize>(), total);
                assert!(counts.iter().all(|&count| count > 0));
                assert!(counts.iter().max().unwrap() - counts.iter().min().unwrap() <= 1);
            }
        }
    }

    #[test]
    fn percentiles_are_nearest_rank_batch_latencies() {
        assert_eq!(percentile(&[1000], 99), 1.0);
        assert_eq!(percentile(&[1000, 2000, 3000, 4000], 50), 2.0);
        assert_eq!(percentile(&[1000, 2000, 3000, 4000], 99), 4.0);
    }

    #[test]
    fn json_strings_escape_controls_quotes_and_backslashes() {
        assert_eq!(quoted("a\"\\\n\r\t\0é"), "\"a\\\"\\\\\\n\\r\\t\\u0000é\"");
    }

    #[test]
    fn incorrect_replies_fail_instead_of_becoming_performance_results() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.read_exact(&mut [0u8; 14]).unwrap();
            stream.write_all(b"+NOPE\r\n").unwrap();
        });
        let commands = Commands::new(Workload::Ping, &Options::default());
        let error = Connection::connect(addr)
            .unwrap()
            .exchange(&commands, 0, 1)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        server.join().unwrap();
    }

    #[test]
    fn conditional_workloads_encode_the_real_options_and_expected_replies() {
        let options = Options {
            keyspace: 1,
            value_size: 7,
            ..Options::default()
        };
        for (workload, option, reply) in [
            (
                Workload::SetNxExisting,
                b"NX".as_slice(),
                b"$-1\r\n".as_slice(),
            ),
            (
                Workload::SetXxExisting,
                b"XX".as_slice(),
                b"+OK\r\n".as_slice(),
            ),
        ] {
            let commands = Commands::new(workload, &options);
            let parsed = rillcache::protocol::parse_request(
                &commands.encoded[0],
                rillcache::protocol::Limits::default(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                parsed.arguments,
                [
                    b"SET".as_slice(),
                    b"rillcache:bench:000000000000",
                    b"xxxxxxx",
                    option
                ]
            );
            assert_eq!(commands.expected, reply);
        }
    }

    #[test]
    fn conditional_benchmarks_allow_sparse_access_without_warmup() {
        let options = Options {
            requests: 1,
            clients: 1,
            pipeline: 16,
            keyspace: 17,
            value_size: 7,
            warmup: 0,
            runs: 1,
            workloads: vec![Workload::SetNxExisting, Workload::SetXxExisting],
            ..Options::default()
        };
        let json = benchmark(&options).unwrap();
        assert_eq!(json.matches("\"requests\":1,").count(), 2);
        assert!(json.contains("\"workload\":\"set-nx-existing\""));
        assert!(json.contains("\"workload\":\"set-xx-existing\""));
    }

    #[test]
    fn report_uses_captured_scheduling_policy_without_resampling_capacity() {
        let options = Options::default();
        let policy = rillcache::server::SchedulingPolicy {
            readiness_io: false,
            tokio_transport: false,
            tokio_worker_threads: 0,
            tokio_compatibility_owner: false,
            active_poll_us: 0,
            run_quantum_us: 0,
            single_reply_yield: true,
            client_handoff_idle_us: rillcache::server::CLIENT_HANDOFF_IDLE_US,
            client_handoff_lanes: 1,
        };
        let text = report(
            "127.0.0.1:1".parse().unwrap(),
            &options,
            &[],
            None,
            Some(policy),
        );
        assert!(text.contains("\"single_reply_yield\":true"));
        assert!(text.contains("\"client_handoff_idle_us\":50"));
        assert!(text.contains("\"client_handoff_policy_version\":2"));
        assert!(text.contains("\"client_handoff_max_epoch_ms\":100"));
        assert!(text.contains("\"client_handoff_lanes\":1"));
        let multicore = rillcache::server::SchedulingPolicy {
            single_reply_yield: false,
            client_handoff_idle_us: rillcache::server::CLIENT_HANDOFF_MULTICORE_IDLE_US,
            client_handoff_lanes: 2,
            ..policy
        };
        let text = report(
            "127.0.0.1:1".parse().unwrap(),
            &options,
            &[],
            None,
            Some(multicore),
        );
        assert!(text.contains("\"single_reply_yield\":false"));
        assert!(text.contains("\"client_handoff_idle_us\":100"));
        assert!(text.contains("\"client_handoff_policy_version\":8"));
        assert!(text.contains("\"client_handoff_lanes\":2"));
        assert!(text.contains("\"client_handoff_max_epoch_ms\":100"));
        let policy = rillcache::server::SchedulingPolicy {
            single_reply_yield: false,
            client_handoff_idle_us: 0,
            client_handoff_lanes: 0,
            ..policy
        };
        let text = report(
            "127.0.0.1:1".parse().unwrap(),
            &options,
            &[],
            None,
            Some(policy),
        );
        assert!(text.contains("\"single_reply_yield\":false"));
        assert!(text.contains("\"client_handoff_idle_us\":0"));
        assert!(text.contains("\"client_handoff_policy_version\":0"));
        assert!(text.contains("\"client_handoff_max_epoch_ms\":0"));
        assert!(text.contains("\"client_handoff_lanes\":0"));
        let mio = rillcache::server::SchedulingPolicy {
            readiness_io: true,
            tokio_transport: false,
            ..policy
        };
        let text = report(
            "127.0.0.1:1".parse().unwrap(),
            &options,
            &[],
            None,
            Some(mio),
        );
        assert!(text.contains("\"mode\":\"mio-readiness\""));
        assert!(!text.contains("network_runtime_threads"));
        let readiness = rillcache::server::SchedulingPolicy {
            readiness_io: true,
            tokio_transport: true,
            tokio_worker_threads: 3,
            ..policy
        };
        let text = report(
            "127.0.0.1:1".parse().unwrap(),
            &options,
            &[],
            None,
            Some(readiness),
        );
        assert!(text.contains("\"mode\":\"tokio-current-thread\""));
        assert!(text.contains("\"workers\":3"));
        assert!(text.contains("\"network_runtime_threads\":3"));
        assert!(text.contains("\"accept_runtime_shared\":true"));
        assert!(text.contains("\"cold_io_service_threads\":1"));
        assert!(text.contains("\"storage_execution\":\"local-owner\""));
        assert!(text.contains("\"compatibility_data_owner_threads\":0"));
        let compatibility = rillcache::server::SchedulingPolicy {
            tokio_compatibility_owner: true,
            ..readiness
        };
        let text = report(
            "127.0.0.1:1".parse().unwrap(),
            &options,
            &[],
            None,
            Some(compatibility),
        );
        assert!(text.contains("\"storage_execution\":\"compatibility-bridge\""));
        assert!(text.contains("\"compatibility_data_owner_threads\":1"));
        assert!(!text.contains("\"mode\":\"thread-per-client\""));
        assert!(!text.contains("\"client_handoff_policy_version\""));
    }

    #[test]
    fn embedded_benchmark_validates_non_divisible_batches_and_empty_values() {
        let options = Options {
            requests: 19,
            clients: 4,
            pipeline: 3,
            keyspace: 7,
            value_size: 0,
            warmup: 2,
            runs: 1,
            ..Options::default()
        };
        for workers in [None, Some(1), Some(4)] {
            let options = Options {
                server_threads: workers,
                ..options.clone()
            };
            let json = benchmark(&options).unwrap();
            assert_eq!(json.matches("\"requests\":19").count(), Workload::ALL.len());
            assert_eq!(json.matches("\"batches\":8").count(), Workload::ALL.len());
            assert!(json.contains("\"workload\":\"get\""));
            assert!(json.contains("\"server_configuration\":"));
            assert!(json.starts_with("{\"schema_version\":1,"));
            assert!(json.ends_with("]}"));
        }
    }
}
