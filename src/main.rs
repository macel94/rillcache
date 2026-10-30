#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::Duration;

use rillcache::{Config, Server};

mod healthcheck;

const HELP: &str = "rillcache — safe Rust Redis-compatible server\n\nUsage: rillcache [OPTIONS]\n\n  --bind ADDRESS           Plaintext socket address (default 127.0.0.1:6379)\n  --max-clients N          Maximum simultaneous clients (default 128)\n  --auth-failure-limit N  Failed AUTH attempts per connection before close (default 10; 0 off)\n  --auth-source-limit N   Failed AUTH attempts per peer IP before lockout (default 0; 0 off)\n  --auth-source-window-ms N  Source failure window and lockout duration (default 60000)\n  --threads N              Fixed I/O/execution workers, 1..64 (default: Unix readiness; managed clients elsewhere)\n  --io-mode NAME           Plain-TCP engine: mio (default) or tokio (opt-in async)\n  --storage-owners N       Opt-in multi-owner data execution, 1..64 (default 1: one local owner)\n  --max-data-bytes N       Key/value payload budget (default 67108864)\n  --max-bulk-bytes N       Maximum request argument bytes (default 1048576; max 536870912)\n  --max-frame-bytes N      Maximum request frame bytes (default 2097152; max 536870912)\n  --idle-timeout-ms N      Socket inactivity timeout (default 30000)\n  --requirepass PASSWORD   Default-user password; empty disables (visible in process args)\n  --requirepass-file PATH  Read the password from a file; one trailing newline is stripped\n  --aclfile PATH           Load users/rules from a Redis-style ACL file (instead of --requirepass)\n  --audit-log PATH         Append ACL denials to a bounded audit file (default off)\n  --audit-log-max-bytes N  One-generation audit rotation bound (default 8388608)\n  --audit-syslog ADDRESS   Best-effort UDP destination for denial lines (default off)\n  --audit-hmac-key-file PATH  Optional 16..=1024-byte key for per-line HMAC (default off)\n  --audit-hmac-keyring-file PATH  Hex keyring; first key signs, all verify (default off)\n  --appendonly yes|no      Append-only persistence (default no)\n  --appendfilename NAME    Journal basename inside --dir (default appendonly.aof)\n  --appendfsync POLICY     always, everysec or no (default everysec)\n  --dir PATH               Directory for the journal (default .)\n  --maxmemory-policy NAME  noeviction, allkeys-lru/random or volatile-lru/random/ttl\n  --notify-keyspace-events FLAGS  K/E/g/$/l/s/h/z/t/A notification classes\n  --error-dialect NAME     redis or valkey error wording (default valkey; MSETEX short pairs)\n  --tls-bind ADDRESS       Optional native TLS 1.3 listener (distinct from --bind)\n  --tls-cert-file PATH     PEM server certificate chain (required with --tls-bind)\n  --tls-key-file PATH      PEM server private key (required with --tls-bind)\n  --tls-client-ca-file PATH  Optional PEM client CA; requires client certificates\n  --tls-handshake-timeout-ms N  TLS handshake deadline (default 5000)\n  --healthcheck IP:PORT    Read-only PING probe, one-second deadline; exit 0/1\n  --help                   Show this help\n  --version                Show version\n\nExperimental: password/ACL-file AUTH, native TLS 1.3 via rustls, eviction and an\nappend-only journal; no RDB format, replication or HTTP protocol. The standalone\nbinary binds to loopback by default.";

#[allow(clippy::large_enum_variant)]
enum Options {
    Run(Config),
    Help,
    Version,
    Healthcheck(SocketAddr),
}

fn options(arguments: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut config = Config::default();
    let mut arguments = arguments.into_iter();
    while let Some(option) = arguments.next() {
        match option.as_str() {
            "--help" | "-h" => return Ok(Options::Help),
            "--version" | "-V" => return Ok(Options::Version),
            "--healthcheck" => {
                let address = arguments
                    .next()
                    .ok_or("missing value for --healthcheck")?
                    .parse::<SocketAddr>()
                    .ok()
                    .filter(|address| address.port() != 0)
                    .ok_or("--healthcheck requires a numeric IP:port with a nonzero port")?;
                if arguments.next().is_some() {
                    return Err("unexpected arguments after --healthcheck".into());
                }
                return Ok(Options::Healthcheck(address));
            }
            "--bind"
            | "--max-clients"
            | "--auth-failure-limit"
            | "--auth-source-limit"
            | "--auth-source-window-ms"
            | "--threads"
            | "--io-mode"
            | "--storage-owners"
            | "--max-data-bytes"
            | "--max-bulk-bytes"
            | "--max-frame-bytes"
            | "--idle-timeout-ms"
            | "--requirepass"
            | "--requirepass-file"
            | "--aclfile"
            | "--audit-log"
            | "--audit-log-max-bytes"
            | "--audit-syslog"
            | "--audit-hmac-key-file"
            | "--audit-hmac-keyring-file"
            | "--appendonly"
            | "--appendfilename"
            | "--appendfsync"
            | "--dir"
            | "--maxmemory-policy"
            | "--notify-keyspace-events"
            | "--error-dialect"
            | "--tls-bind"
            | "--tls-cert-file"
            | "--tls-key-file"
            | "--tls-client-ca-file"
            | "--tls-handshake-timeout-ms" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| format!("missing value for {option}"))?;
                match option.as_str() {
                    "--bind" => {
                        config.bind_addr = value
                            .parse()
                            .map_err(|_| "--bind requires a numeric IP:port address")?
                    }
                    "--threads" => {
                        let count = positive(&option, &value)?;
                        if count > 64 {
                            return Err("--threads must be in 1..=64".into());
                        }
                        config.worker_threads = Some(count);
                    }
                    "--io-mode" => {
                        config.io_mode = match value.as_str() {
                            "mio" => rillcache::server::IoMode::Mio,
                            "tokio" => rillcache::server::IoMode::Tokio,
                            _ => return Err("--io-mode requires mio or tokio".into()),
                        };
                    }
                    "--storage-owners" => {
                        let count = positive(&option, &value)?;
                        if !(1..=64).contains(&count) {
                            return Err("--storage-owners must be in 1..=64".into());
                        }
                        config.storage_owners = Some(count);
                    }
                    "--max-clients" => config.max_clients = positive(&option, &value)?,
                    "--auth-failure-limit" => {
                        config.auth_failure_limit = value
                            .parse::<usize>()
                            .ok()
                            .filter(|limit| *limit <= 10_000)
                            .ok_or("--auth-failure-limit requires 0..=10000")?;
                    }
                    "--auth-source-limit" => {
                        config.auth_source_limit = value
                            .parse::<usize>()
                            .ok()
                            .filter(|limit| *limit <= 10_000)
                            .ok_or("--auth-source-limit requires 0..=10000")?;
                    }
                    "--auth-source-window-ms" => {
                        config.auth_source_window_ms = value
                            .parse::<u64>()
                            .ok()
                            .filter(|window| *window > 0)
                            .ok_or("--auth-source-window-ms requires a positive integer")?;
                    }
                    "--max-data-bytes" => config.max_data_bytes = positive(&option, &value)?,
                    "--max-bulk-bytes" => {
                        config.limits.max_bulk_bytes = finite_limit(&option, &value)?
                    }
                    "--max-frame-bytes" => {
                        config.limits.max_frame_bytes = finite_limit(&option, &value)?
                    }
                    "--idle-timeout-ms" => {
                        config.idle_timeout = Duration::from_millis(
                            value
                                .parse::<u64>()
                                .ok()
                                .filter(|&n| n > 0)
                                .ok_or("--idle-timeout-ms requires a positive integer")?,
                        )
                    }
                    "--requirepass" => {
                        config.requirepass = (!value.is_empty()).then(|| value.into_bytes());
                    }
                    "--requirepass-file" => {
                        config.requirepass = Some(read_password_file(&value)?);
                    }
                    "--aclfile" => {
                        if value.is_empty() {
                            return Err("--aclfile requires a nonempty path".into());
                        }
                        config.aclfile = Some(std::path::PathBuf::from(value));
                    }
                    "--audit-log" => {
                        if value.is_empty() {
                            return Err("--audit-log requires a nonempty path".into());
                        }
                        config.audit_log = Some(std::path::PathBuf::from(value));
                    }
                    "--audit-log-max-bytes" => {
                        config.audit_log_max_bytes = value
                            .parse::<u64>()
                            .ok()
                            .filter(|bytes| (1024..=1024 * 1024 * 1024).contains(bytes))
                            .ok_or("--audit-log-max-bytes requires 1024..=1073741824")?;
                    }
                    "--audit-syslog" => {
                        config.audit_syslog = Some(
                            value
                                .parse::<std::net::SocketAddr>()
                                .ok()
                                .filter(|address| address.port() != 0)
                                .ok_or(
                                    "--audit-syslog requires a numeric IP:port with a nonzero port",
                                )?,
                        );
                    }
                    "--audit-hmac-key-file" => {
                        if value.is_empty() {
                            return Err("--audit-hmac-key-file requires a nonempty path".into());
                        }
                        config.audit_hmac_key_file = Some(std::path::PathBuf::from(value));
                    }
                    "--audit-hmac-keyring-file" => {
                        if value.is_empty() {
                            return Err("--audit-hmac-keyring-file requires a nonempty path".into());
                        }
                        config.audit_hmac_keyring_file = Some(std::path::PathBuf::from(value));
                    }
                    "--appendonly" => {
                        config.appendonly = match value.as_str() {
                            "yes" => true,
                            "no" => false,
                            _ => return Err("--appendonly requires yes or no".into()),
                        };
                    }
                    "--appendfilename" => {
                        if value.is_empty() {
                            return Err("--appendfilename requires a nonempty basename".into());
                        }
                        config.appendfilename = value;
                    }
                    "--appendfsync" => {
                        config.appendfsync = rillcache::engine::FsyncPolicy::parse(&value)
                            .ok_or("--appendfsync requires always, everysec or no")?;
                    }
                    "--dir" => {
                        if value.is_empty() {
                            return Err("--dir requires a nonempty path".into());
                        }
                        config.dir = std::path::PathBuf::from(value);
                    }
                    "--maxmemory-policy" => {
                        config.maxmemory_policy = rillcache::engine::EvictionPolicy::parse(&value)
                            .ok_or("--maxmemory-policy requires a supported policy name")?;
                    }
                    "--error-dialect" => {
                        config.error_dialect = rillcache::engine::ErrorDialect::parse(&value)
                            .ok_or("--error-dialect requires redis or valkey")?;
                    }
                    "--notify-keyspace-events" => {
                        config.notify_keyspace_events =
                            rillcache::engine::NotifyFlags::parse(value.as_bytes())
                                .ok_or("--notify-keyspace-events requires supported classes")?;
                    }
                    "--tls-bind" => {
                        let address = value
                            .parse::<SocketAddr>()
                            .map_err(|_| "--tls-bind requires a numeric IP:port address")?;
                        if address == config.bind_addr {
                            return Err("--tls-bind must differ from --bind".into());
                        }
                        config.tls_bind_addr = Some(address);
                    }
                    "--tls-cert-file" => config.tls_cert_file = Some(value.into()),
                    "--tls-key-file" => config.tls_key_file = Some(value.into()),
                    "--tls-client-ca-file" => config.tls_client_ca_file = Some(value.into()),
                    "--tls-handshake-timeout-ms" => {
                        config.tls_handshake_timeout =
                            Duration::from_millis(positive(&option, &value)? as u64)
                    }
                    _ => unreachable!(),
                }
            }
            _ => return Err(format!("unknown option: {option}")),
        }
    }
    Ok(Options::Run(config))
}

/// Read a secret file, stripping at most one trailing line ending. An empty or
/// oversized file is an error rather than silently disabling authentication.
fn read_password_file(path: &str) -> Result<Vec<u8>, String> {
    let mut bytes = std::fs::read(path)
        .map_err(|error| format!("--requirepass-file could not read {path}: {error}"))?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    if bytes.is_empty() {
        return Err("--requirepass-file must contain a nonempty password".into());
    }
    if bytes.len() > 1024 {
        return Err("--requirepass-file must not exceed 1024 bytes".into());
    }
    Ok(bytes)
}

fn finite_limit(option: &str, value: &str) -> Result<usize, String> {
    positive(option, value).and_then(|n| {
        (n <= 512 * 1024 * 1024)
            .then_some(n)
            .ok_or_else(|| format!("{option} must not exceed 536870912 bytes"))
    })
}

fn positive(option: &str, value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|&n| n > 0)
        .ok_or_else(|| format!("{option} requires a positive integer"))
}

fn main() -> ExitCode {
    match options(std::env::args().skip(1)) {
        Ok(Options::Help) => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Options::Version) => {
            println!("rillcache {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Options::Healthcheck(address)) => match healthcheck::check(address) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("rillcache: health check failed: {error}");
                ExitCode::FAILURE
            }
        },
        Ok(Options::Run(config)) => match Server::bind(config).and_then(|server| {
            eprintln!(
                "rillcache {} listening on {}",
                env!("CARGO_PKG_VERSION"),
                server.local_addr()?
            );
            if let Some(addr) = server.tls_local_addr()? {
                eprintln!("rillcache TLS 1.3 listening on {addr}");
            }
            server.run()
        }) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("rillcache: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("rillcache: {error}\nUse --help for usage.");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_options_parse_and_require_a_distinct_listener() {
        let Options::Run(config) = options(
            [
                "--tls-bind",
                "127.0.0.1:6380",
                "--tls-cert-file",
                "server.pem",
                "--tls-key-file",
                "server.key",
                "--tls-client-ca-file",
                "ca.pem",
                "--tls-handshake-timeout-ms",
                "2500",
            ]
            .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected TLS config");
        };
        assert_eq!(
            config.tls_bind_addr,
            Some("127.0.0.1:6380".parse().unwrap())
        );
        assert_eq!(
            config.tls_cert_file.as_deref(),
            Some(std::path::Path::new("server.pem"))
        );
        assert_eq!(
            config.tls_key_file.as_deref(),
            Some(std::path::Path::new("server.key"))
        );
        assert_eq!(
            config.tls_client_ca_file.as_deref(),
            Some(std::path::Path::new("ca.pem"))
        );
        assert_eq!(config.tls_handshake_timeout, Duration::from_millis(2500));
        assert!(
            options(["--tls-bind", "127.0.0.1:6379"].map(str::to_owned)).is_err(),
            "TLS listener must differ from the plaintext listener"
        );
    }

    #[test]
    fn standalone_defaults_and_appended_container_options_are_distinct() {
        let Options::Run(defaults) = options(Vec::new()).unwrap() else {
            panic!("expected defaults");
        };
        assert!(defaults.bind_addr.ip().is_loopback());
        let Options::Run(container) =
            options(["--bind", "0.0.0.0:6379", "--threads", "4"].map(str::to_owned)).unwrap()
        else {
            panic!("expected container config");
        };
        assert!(container.bind_addr.ip().is_unspecified());
        assert_eq!(container.worker_threads, Some(4));
        let Options::Run(authenticated) =
            options(["--requirepass", "s3cret", "--requirepass", ""].map(str::to_owned)).unwrap()
        else {
            panic!("expected authentication config");
        };
        assert_eq!(authenticated.requirepass, None);
        let Options::Run(authenticated) =
            options(["--requirepass", "s3cret"].map(str::to_owned)).unwrap()
        else {
            panic!("expected authentication config");
        };
        assert_eq!(
            authenticated.requirepass.as_deref(),
            Some(b"s3cret".as_slice())
        );
        assert!(!format!("{authenticated:?}").contains("s3cret"));
        let path = std::env::temp_dir().join(format!(
            "rillcache-password-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, b"from-file\r\n").unwrap();
        let Options::Run(authenticated) =
            options(["--requirepass-file", path.to_str().unwrap()].map(str::to_owned)).unwrap()
        else {
            panic!("expected file authentication config");
        };
        assert_eq!(
            authenticated.requirepass.as_deref(),
            Some(b"from-file".as_slice())
        );
        std::fs::write(&path, b"\n").unwrap();
        assert!(
            options(["--requirepass-file", path.to_str().unwrap()].map(str::to_owned)).is_err()
        );
        std::fs::write(&path, vec![b'x'; 1025]).unwrap();
        assert!(
            options(["--requirepass-file", path.to_str().unwrap()].map(str::to_owned)).is_err()
        );
        std::fs::remove_file(&path).unwrap();
        let Options::Run(journal) = options(
            [
                "--appendonly",
                "yes",
                "--appendfilename",
                "test.aof",
                "--appendfsync",
                "always",
                "--dir",
                "/data",
            ]
            .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected journal config");
        };
        assert!(journal.appendonly);
        assert_eq!(journal.appendfilename, "test.aof");
        assert_eq!(journal.appendfsync, rillcache::engine::FsyncPolicy::Always);
        assert_eq!(journal.dir, std::path::Path::new("/data"));
        let Options::Run(eviction) =
            options(["--maxmemory-policy", "allkeys-lru"].map(str::to_owned)).unwrap()
        else {
            panic!("expected eviction config");
        };
        assert_eq!(
            eviction.maxmemory_policy,
            rillcache::engine::EvictionPolicy::AllKeysLru
        );
        let Options::Run(notify) =
            options(["--notify-keyspace-events", "KEA"].map(str::to_owned)).unwrap()
        else {
            panic!("expected notification config");
        };
        assert_eq!(notify.notify_keyspace_events.as_string(), "KEg$lshztxe");
        let Options::Run(acl) =
            options(["--aclfile", "/etc/rillcache/users.acl"].map(str::to_owned)).unwrap()
        else {
            panic!("expected ACL config");
        };
        assert_eq!(
            acl.aclfile.as_deref(),
            Some(std::path::Path::new("/etc/rillcache/users.acl"))
        );
        let Options::Run(audit) = options(
            [
                "--audit-log",
                "/var/log/rillcache/acl-audit.log",
                "--audit-log-max-bytes",
                "4096",
                "--audit-syslog",
                "127.0.0.1:5514",
                "--audit-hmac-key-file",
                "/run/secrets/audit.key",
                "--audit-hmac-keyring-file",
                "/run/secrets/audit.keyring",
            ]
            .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected audit config");
        };
        assert_eq!(
            audit.audit_log.as_deref(),
            Some(std::path::Path::new("/var/log/rillcache/acl-audit.log"))
        );
        assert_eq!(audit.audit_log_max_bytes, 4096);
        assert_eq!(audit.audit_syslog, Some("127.0.0.1:5514".parse().unwrap()));
        assert_eq!(
            audit.audit_hmac_key_file.as_deref(),
            Some(std::path::Path::new("/run/secrets/audit.key"))
        );
        assert_eq!(
            audit.audit_hmac_keyring_file.as_deref(),
            Some(std::path::Path::new("/run/secrets/audit.keyring"))
        );
        assert!(options(["--audit-log-max-bytes", "1023"].map(str::to_owned)).is_err());
        assert!(options(["--audit-syslog", "localhost:514"].map(str::to_owned)).is_err());
        assert!(options(["--audit-syslog", "127.0.0.1:0"].map(str::to_owned)).is_err());
        assert!(options(["--audit-hmac-key-file", ""].map(str::to_owned)).is_err());
        assert!(options(["--audit-hmac-keyring-file", ""].map(str::to_owned)).is_err());
        let Options::Run(overridden) =
            options(["--bind", "0.0.0.0:6379", "--bind", "127.0.0.1:26379"].map(str::to_owned))
                .unwrap()
        else {
            panic!("expected override");
        };
        assert_eq!(overridden.bind_addr, "127.0.0.1:26379".parse().unwrap());
        let Options::Healthcheck(address) =
            options(["--bind", "0.0.0.0:6379", "--healthcheck", "[::1]:6379"].map(str::to_owned))
                .unwrap()
        else {
            panic!("expected health check");
        };
        assert_eq!(address, "[::1]:6379".parse().unwrap());
    }

    #[test]
    fn error_dialect_is_explicit_and_validated_at_startup() {
        use rillcache::engine::ErrorDialect;
        let Options::Run(defaults) = options(Vec::new()).unwrap() else {
            panic!("expected defaults");
        };
        assert_eq!(defaults.error_dialect, ErrorDialect::Valkey);
        for (name, expected) in [
            ("redis", ErrorDialect::Redis),
            ("valkey", ErrorDialect::Valkey),
        ] {
            let Options::Run(config) =
                options(["--error-dialect", name].map(str::to_owned)).unwrap()
            else {
                panic!("expected dialect config");
            };
            assert_eq!(config.error_dialect, expected);
        }
        for arguments in [
            vec!["--error-dialect"],
            vec!["--error-dialect", ""],
            vec!["--error-dialect", "auto"],
            vec!["--error-dialect", "redis\0"],
            vec!["--error-dialect", "REDIS"],
        ] {
            assert!(options(arguments.into_iter().map(str::to_owned)).is_err());
        }
    }

    #[test]
    fn cli_validates_arguments() {
        for arguments in [
            vec!["--unknown"],
            vec!["--bind"],
            vec!["--bind", "localhost:6379"],
            vec!["--max-clients", "0"],
            vec!["--auth-failure-limit", "10001"],
            vec!["--auth-failure-limit", "-1"],
            vec!["--auth-failure-limit", "x"],
            vec!["--auth-source-limit", "10001"],
            vec!["--auth-source-limit", "-1"],
            vec!["--auth-source-limit", "x"],
            vec!["--auth-source-window-ms", "0"],
            vec!["--auth-source-window-ms", "-1"],
            vec!["--threads", "0"],
            vec!["--threads", "65"],
            vec!["--threads", "-1"],
            vec!["--max-data-bytes", "-1"],
            vec!["--max-bulk-bytes", "0"],
            vec!["--max-frame-bytes", "0"],
            vec!["--max-frame-bytes", "536870913"],
            vec!["--max-bulk-bytes", "536870913"],
            vec!["--idle-timeout-ms", "0"],
            vec!["--healthcheck"],
            vec!["--healthcheck", "localhost:6379"],
            vec!["--healthcheck", "127.0.0.1:0"],
            vec!["--healthcheck", "127.0.0.1:6379", "--unknown"],
            vec!["--requirepass"],
            vec!["--requirepass-file"],
            vec![
                "--requirepass-file",
                "/definitely/missing/rillcache-password",
            ],
        ] {
            assert!(options(arguments.into_iter().map(str::to_owned)).is_err());
        }
        let Options::Run(config) = options(
            [
                "--bind",
                "127.0.0.1:0",
                "--threads",
                "4",
                "--max-clients",
                "4",
                "--auth-failure-limit",
                "3",
                "--auth-source-limit",
                "5",
                "--auth-source-window-ms",
                "250",
                "--max-data-bytes",
                "100",
                "--max-bulk-bytes",
                "8192",
                "--max-frame-bytes",
                "16384",
                "--idle-timeout-ms",
                "250",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected configuration");
        };
        assert_eq!(config.bind_addr.port(), 0);
        assert_eq!(config.max_clients, 4);
        assert_eq!(config.auth_failure_limit, 3);
        assert_eq!(config.auth_source_limit, 5);
        assert_eq!(config.auth_source_window_ms, 250);
        assert_eq!(config.worker_threads, Some(4));
        assert_eq!(config.max_data_bytes, 100);
        assert_eq!(config.limits.max_bulk_bytes, 8192);
        assert_eq!(config.limits.max_frame_bytes, 16384);
        assert_eq!(config.idle_timeout, Duration::from_millis(250));
    }
}
