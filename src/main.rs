#![forbid(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;

use rillcache::protocol::Limits;
use rillcache::{Config, Server, TlsSniCertificate};

mod config_file;
mod healthcheck;

type SniFiles = (
    Option<std::path::PathBuf>,
    Option<std::path::PathBuf>,
    Option<std::path::PathBuf>,
);

const HELP: &str = "rillcache — safe Rust Redis-compatible server\n\nUsage: rillcache [OPTIONS]\n\n  --bind ADDRESS           Plaintext socket address (default 127.0.0.1:6379)\n  --max-clients N          Maximum simultaneous clients (default 128)\n  --auth-failure-limit N  Failed AUTH attempts per connection before close (default 10; 0 off)\n  --auth-source-limit N   Failed AUTH attempts per peer IP before lockout (default 0; 0 off)\n  --auth-global-limit N   Failed AUTH attempts process-wide before lockout (default 0; 0 off)\n  --auth-source-window-ms N  Source/global failure window and lockout duration (default 60000)\n  --threads N              Fixed I/O/execution workers, 1..64 (default: Unix readiness; managed clients elsewhere)\n  --io-mode NAME           Plain-TCP engine: mio (default) or tokio (opt-in async)\n  --storage-owners N       Opt-in multi-owner data execution, 1..64 (default 1: one local owner)
  --socket-workers N       Plain-TCP reactors, 1..16 (default: 1 owner-local reactor; 2..=16 spread connection-local commands and migrate data connections to the owner)\n  --max-data-bytes N       Key/value payload budget (default 67108864)\n  --max-bulk-bytes N       Maximum request argument bytes (default 1048576; max 536870912)\n  --max-frame-bytes N      Maximum request frame bytes (default 2097152; max 536870912); the argument cap follows frame/4, clamped to 1024..1048576\n  --idle-timeout-ms N      Socket inactivity timeout (default 30000)
  --shutdown-deadline-ms N  Grace period to join workers after SIGTERM/SIGINT before forcing exit (default 25000; max 600000)\n  --frame-assembly-timeout-ms N  Incomplete plaintext frame deadline (default 60000; 0 disables)\n  --write-stall-timeout-ms N  Pending-reply write-stall deadline (default 30000; 0 disables)\n  --global-command-budget N  Process-wide command budget per second (default 0: off)\n  --user-command-budget N   Per-authenticated-user command budget per second (default 0: off)\n  --requirepass PASSWORD   Default-user password; empty disables (visible in process args)\n  --requirepass-file PATH  Read the password from a file; one trailing newline is stripped\n  --secure-secret-files[=yes|no]  Opt in to owner-only secret file permissions (Unix); bare flag means yes\n  --aclfile PATH           Load users/rules from a Redis-style ACL file (instead of --requirepass)\n  --audit-log PATH         Append ACL denials to a bounded audit file (default off)\n  --audit-log-max-bytes N  One-generation audit rotation bound (default 8388608)\n  --audit-syslog ADDRESS   Best-effort UDP destination for denial lines (default off)\n  --audit-hmac-key-file PATH  Optional 16..=1024-byte key for per-line HMAC (default off)\n  --audit-hmac-keyring-file PATH  Hex keyring; first key signs, all verify (default off)\n  --appendonly yes|no      Append-only persistence (default no)\n  --appendfilename NAME    Journal basename inside --dir (default appendonly.aof)\n  --appendfsync POLICY     always, everysec or no (default everysec)\n  --aof-hmac-key-file PATH  HMAC-SHA256 authenticate journal transactions (default off)\n  --aof-hmac-migrate      One-time seal of an existing journal; requires the key
  --aof-anchor-file PATH  Read-only rollback/truncation anchor required for authenticated startup\n  --dir PATH               Directory for the journal (default .)\n  --maxmemory-policy NAME  noeviction, allkeys-lru/random or volatile-lru/random/ttl\n  --notify-keyspace-events FLAGS  K/E/g/$/l/s/h/z/t/m/n/A notification classes\n  --error-dialect NAME     redis or valkey error wording (default valkey; selected MSETEX/HGETDEL errors)\n  --tls-bind ADDRESS       Optional native TLS 1.3 listener (distinct from --bind)\n  --tls-cert-file PATH     PEM server certificate chain (required with --tls-bind)\n  --tls-key-file PATH      PEM server private key (required with --tls-bind)\n  --tls-ocsp-file PATH     Optional DER OCSP response stapled with the default identity\n  --tls-client-ca-file PATH  Optional PEM client CA; requires client certificates\n  --tls-client-crl-file PATH  DER client CRL; unknown or expired status fails closed (requires CA)\n  --tls-client-cert-user SHA256=USER  Map a verified leaf fingerprint to an ACL user (repeatable)\n  --tls-client-cert-users-file PATH  Bounded SHA256=ACL-user mapping file, atomically reloaded with TLS generation\n  --tls-reload-trigger PATH  Reload TLS files when this file's token changes\n  --tls-sni-cert-file NAME=PATH  Additional exact SNI certificate chain (repeatable)\n  --tls-sni-key-file NAME=PATH   Private key for the matching SNI name (repeatable)\n  --tls-sni-ocsp-file NAME=PATH  Optional DER OCSP response for that SNI identity (repeatable)\n  --tls-handshake-timeout-ms N  TLS handshake deadline (default 5000)\n  --healthcheck IP:PORT    Read-only PING probe, one-second deadline; exit 0/1\n  --healthcheck-password-file PATH  After --healthcheck: AUTH with password from this file\n  --healthcheck-user NAME  After --healthcheck: ACL user (requires password file)\n  --healthcheck-require-aof  After --healthcheck: require healthy reported journal status\n  --healthcheck-min-payload-headroom BYTES  After --healthcheck: require logical payload headroom\n  --healthcheck-aof-write-probe DIR  With require-aof: test one-byte write/sync/remove in this directory\n  --healthcheck-tls-ca-file PATH  After --healthcheck: TLS 1.3 with explicit PEM trust roots\n  --healthcheck-server-name NAME  TLS certificate identity (default: probe IP)\n  --config PATH            Read startup directives from a bounded file; later CLI options override\n  --check-config           First argument only: validate startup options without binding\n  --help                   Show this help\n  --version                Show version\n\nExperimental: password/ACL-file AUTH, native TLS 1.3 via rustls, eviction and an\nappend-only journal; no RDB format, replication or HTTP protocol. The standalone\nbinary binds to loopback by default.";

#[allow(clippy::large_enum_variant)]
enum Options {
    Run(Config),
    Help,
    Version,
    Healthcheck {
        address: SocketAddr,
        username: Option<Vec<u8>>,
        password: Option<Vec<u8>>,
        require_aof: bool,
        aof_write_probe_directory: Option<std::path::PathBuf>,
        min_payload_headroom: Option<u64>,
        tls: Option<healthcheck::TlsSettings>,
    },
}

fn parse_tls_client_cert_user(value: &str) -> Result<([u8; 32], Vec<u8>), String> {
    let (fingerprint, username) = value
        .split_once('=')
        .ok_or("--tls-client-cert-user requires SHA256=ACL-USERNAME")?;
    if fingerprint.len() != 64
        || username.is_empty()
        || username.len() > 128
        || !username
            .bytes()
            .all(|byte| (b'!'..=b'~').contains(&byte) && byte != b'|')
    {
        return Err(
            "--tls-client-cert-user requires a 64-digit SHA-256 fingerprint and valid ACL username"
                .into(),
        );
    }
    let mut digest = [0u8; 32];
    for (index, chunk) in fingerprint.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let pair = std::str::from_utf8(chunk).map_err(|_| "invalid SHA-256 fingerprint")?;
        digest[index] = u8::from_str_radix(pair, 16)
            .map_err(|_| "TLS client certificate fingerprint must be hexadecimal")?;
    }
    Ok((digest, username.as_bytes().to_vec()))
}

fn options(arguments: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut config = Config::default();
    let mut sni_files: BTreeMap<String, SniFiles> = BTreeMap::new();
    let mut tls_client_cert_users = BTreeMap::new();
    let mut arguments: VecDeque<String> = arguments.into_iter().collect();
    let mut remaining_config_bytes = config_file::MAX_TOTAL_CONFIG_BYTES;
    while let Some(option) = arguments.pop_front() {
        if option == "--config" || option == "-c" {
            let path = arguments.pop_front().ok_or("missing value for --config")?;
            let file_arguments = config_file::load(&path, &mut remaining_config_bytes)?;
            for argument in file_arguments.into_iter().rev() {
                arguments.push_front(argument);
            }
            continue;
        }
        match option.as_str() {
            "--help" | "-h" => return Ok(Options::Help),
            "--version" | "-V" => return Ok(Options::Version),
            "--healthcheck" => {
                let address = arguments
                    .pop_front()
                    .ok_or("missing value for --healthcheck")?
                    .parse::<SocketAddr>()
                    .ok()
                    .filter(|address| address.port() != 0)
                    .ok_or("--healthcheck requires a numeric IP:port with a nonzero port")?;
                let mut password = None;
                let mut username = None;
                let mut require_aof = false;
                let mut aof_write_probe_directory = None;
                let mut min_payload_headroom = None;
                let mut ca_file = None;
                let mut server_name = None;
                while let Some(flag) = arguments.pop_front() {
                    match flag.as_str() {
                        "--healthcheck-tls-ca-file" if ca_file.is_none() => {
                            ca_file = Some(
                                arguments
                                    .pop_front()
                                    .ok_or("missing value for --healthcheck-tls-ca-file")?,
                            );
                        }
                        "--healthcheck-server-name" if server_name.is_none() => {
                            server_name = Some(
                                arguments
                                    .pop_front()
                                    .ok_or("missing value for --healthcheck-server-name")?,
                            );
                        }
                        "--healthcheck-require-aof" if !require_aof => require_aof = true,
                        "--healthcheck-min-payload-headroom" if min_payload_headroom.is_none() => {
                            let value = arguments
                                .pop_front()
                                .ok_or("missing value for --healthcheck-min-payload-headroom")?;
                            let minimum = value.parse::<u64>().ok().filter(|_| value.bytes().all(|byte| byte.is_ascii_digit())).ok_or("--healthcheck-min-payload-headroom requires an unsigned byte count")?;
                            min_payload_headroom = Some(minimum);
                        }
                        "--healthcheck-aof-write-probe" if aof_write_probe_directory.is_none() => {
                            let path = arguments
                                .pop_front()
                                .ok_or("missing value for --healthcheck-aof-write-probe")?;
                            if path.is_empty() {
                                return Err(
                                    "--healthcheck-aof-write-probe requires a directory".into()
                                );
                            }
                            aof_write_probe_directory = Some(path.into());
                        }
                        "--healthcheck-password-file" if password.is_none() => {
                            let path = arguments
                                .pop_front()
                                .ok_or("missing value for --healthcheck-password-file")?;
                            password =
                                Some(read_password_file(&path, "--healthcheck-password-file")?);
                        }
                        "--healthcheck-user" if username.is_none() => {
                            let value = arguments
                                .pop_front()
                                .ok_or("missing value for --healthcheck-user")?;
                            if value.is_empty()
                                || value.len() > 128
                                || !value
                                    .bytes()
                                    .all(|byte| (b'!'..=b'~').contains(&byte) && byte != b'|')
                            {
                                return Err("--healthcheck-user requires a valid 1..=128-byte ACL user name".into());
                            }
                            username = Some(value.into_bytes());
                        }
                        _ => {
                            return Err(
                                "unexpected or duplicate arguments after --healthcheck".into()
                            );
                        }
                    }
                }
                if username.is_some() && password.is_none() {
                    return Err("--healthcheck-user requires --healthcheck-password-file".into());
                }
                if aof_write_probe_directory.is_some() && !require_aof {
                    return Err(
                        "--healthcheck-aof-write-probe requires --healthcheck-require-aof".into(),
                    );
                }
                if server_name.is_some() && ca_file.is_none() {
                    return Err(
                        "--healthcheck-server-name requires --healthcheck-tls-ca-file".into(),
                    );
                }
                let tls = ca_file
                    .map(|path| healthcheck::TlsSettings::from_ca_file(&path, server_name, address))
                    .transpose()?;
                return Ok(Options::Healthcheck {
                    address,
                    username,
                    password,
                    require_aof,
                    aof_write_probe_directory,
                    min_payload_headroom,
                    tls,
                });
            }
            "--aof-hmac-migrate" => config.aof_hmac_migrate = true,
            "--secure-secret-files" | "--secure-secret-files=yes" => {
                config.secure_secret_files = true;
            }
            "--secure-secret-files=no" => config.secure_secret_files = false,
            "--bind"
            | "--max-clients"
            | "--auth-failure-limit"
            | "--auth-source-limit"
            | "--auth-global-limit"
            | "--auth-source-window-ms"
            | "--threads"
            | "--io-mode"
            | "--storage-owners"
            | "--socket-workers"
            | "--max-data-bytes"
            | "--max-bulk-bytes"
            | "--max-frame-bytes"
            | "--idle-timeout-ms"
            | "--shutdown-deadline-ms"
            | "--frame-assembly-timeout-ms"
            | "--write-stall-timeout-ms"
            | "--global-command-budget"
            | "--user-command-budget"
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
            | "--aof-hmac-key-file"
            | "--aof-anchor-file"
            | "--dir"
            | "--maxmemory-policy"
            | "--notify-keyspace-events"
            | "--error-dialect"
            | "--tls-bind"
            | "--tls-cert-file"
            | "--tls-key-file"
            | "--tls-ocsp-file"
            | "--tls-client-ca-file"
            | "--tls-client-crl-file"
            | "--tls-client-cert-user"
            | "--tls-client-cert-users-file"
            | "--tls-reload-trigger"
            | "--tls-sni-cert-file"
            | "--tls-sni-key-file"
            | "--tls-sni-ocsp-file"
            | "--tls-handshake-timeout-ms" => {
                let value = arguments
                    .pop_front()
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
                    "--socket-workers" => {
                        let count = positive(&option, &value)?;
                        if !(1..=16).contains(&count) {
                            return Err("--socket-workers must be in 1..=16".into());
                        }
                        config.socket_workers = Some(count);
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
                    "--auth-global-limit" => {
                        config.auth_global_limit = value
                            .parse::<usize>()
                            .ok()
                            .filter(|limit| *limit <= 10_000)
                            .ok_or("--auth-global-limit requires 0..=10000")?;
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
                        config.limits.max_frame_bytes = finite_limit(&option, &value)?;
                        config.limits.max_arguments =
                            Limits::argument_cap(config.limits.max_frame_bytes);
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
                    "--shutdown-deadline-ms" => {
                        config.shutdown_deadline = Duration::from_millis(
                            value
                                .parse::<u64>()
                                .ok()
                                .filter(|millis| (1..=600_000).contains(millis))
                                .ok_or("--shutdown-deadline-ms requires 1..=600000")?,
                        );
                    }
                    "--frame-assembly-timeout-ms" => {
                        let millis = value
                            .parse::<u64>()
                            .ok()
                            .filter(|millis| *millis == 0 || (1..=600_000).contains(millis))
                            .ok_or("--frame-assembly-timeout-ms requires 0 or 1..=600000")?;
                        config.frame_assembly_timeout =
                            (millis != 0).then(|| Duration::from_millis(millis));
                    }
                    "--write-stall-timeout-ms" => {
                        let millis = value
                            .parse::<u64>()
                            .ok()
                            .filter(|millis| *millis == 0 || (1..=600_000).contains(millis))
                            .ok_or("--write-stall-timeout-ms requires 0 or 1..=600000")?;
                        config.write_stall_timeout =
                            (millis != 0).then(|| Duration::from_millis(millis));
                    }
                    "--global-command-budget" => {
                        config.global_command_budget = value
                            .parse::<u32>()
                            .ok()
                            .filter(|budget| *budget <= 1_000_000)
                            .ok_or("--global-command-budget requires 0..=1000000")?;
                    }
                    "--user-command-budget" => {
                        config.user_command_budget = value
                            .parse::<u32>()
                            .ok()
                            .filter(|budget| *budget <= 1_000_000)
                            .ok_or("--user-command-budget requires 0..=1000000")?;
                    }
                    "--requirepass" => {
                        config.requirepass = (!value.is_empty()).then(|| value.into_bytes());
                    }
                    "--requirepass-file" => {
                        config.requirepass =
                            Some(read_password_file(&value, "--requirepass-file")?);
                        config.requirepass_file = Some(std::path::PathBuf::from(value));
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
                    "--aof-hmac-key-file" => {
                        if value.is_empty() {
                            return Err("--aof-hmac-key-file requires a nonempty path".into());
                        }
                        config.aof_hmac_key_file = Some(std::path::PathBuf::from(value));
                    }
                    "--aof-anchor-file" => {
                        if value.is_empty() {
                            return Err("--aof-anchor-file requires a nonempty path".into());
                        }
                        config.aof_anchor_file = Some(std::path::PathBuf::from(value));
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
                        config.tls_bind_addr = Some(address);
                    }
                    "--tls-cert-file" => config.tls_cert_file = Some(value.into()),
                    "--tls-key-file" => config.tls_key_file = Some(value.into()),
                    "--tls-ocsp-file" => config.tls_ocsp_file = Some(value.into()),
                    "--tls-client-ca-file" => config.tls_client_ca_file = Some(value.into()),
                    "--tls-client-crl-file" => config.tls_client_crl_file = Some(value.into()),
                    "--tls-client-cert-users-file" => {
                        config.tls_client_cert_users_file = Some(value.into());
                    }
                    "--tls-client-cert-user" => {
                        let (fingerprint, username) = parse_tls_client_cert_user(&value)?;
                        if tls_client_cert_users
                            .insert(fingerprint, username)
                            .is_some()
                        {
                            return Err("duplicate --tls-client-cert-user fingerprint".into());
                        }
                    }
                    "--tls-reload-trigger" => config.tls_reload_trigger = Some(value.into()),
                    "--tls-sni-cert-file" | "--tls-sni-key-file" | "--tls-sni-ocsp-file" => {
                        let (name, path) = value
                            .split_once('=')
                            .ok_or_else(|| format!("{option} requires NAME=PATH"))?;
                        if name.is_empty() || path.is_empty() {
                            return Err(format!("{option} requires a nonempty NAME=PATH"));
                        }
                        let files = sni_files.entry(name.to_ascii_lowercase()).or_default();
                        let target = match option.as_str() {
                            "--tls-sni-cert-file" => &mut files.0,
                            "--tls-sni-key-file" => &mut files.1,
                            _ => &mut files.2,
                        };
                        if target.replace(path.into()).is_some() {
                            return Err(format!("duplicate {option} for SNI name {name}"));
                        }
                    }
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
    // Validate the final listener pair, after applying positional overrides.
    // Equal port-zero requests are two independent kernel binds.
    if config
        .tls_bind_addr
        .is_some_and(|address| address == config.bind_addr && address.port() != 0)
    {
        return Err("--tls-bind must differ from --bind".into());
    }
    config.tls_client_cert_users = tls_client_cert_users;
    for (server_name, (cert_file, key_file, ocsp_file)) in sni_files {
        let cert_file = cert_file
            .ok_or_else(|| format!("missing --tls-sni-cert-file for SNI name {server_name}"))?;
        let key_file = key_file
            .ok_or_else(|| format!("missing --tls-sni-key-file for SNI name {server_name}"))?;
        config.tls_sni_certificates.push(TlsSniCertificate {
            server_name,
            cert_file,
            key_file,
            ocsp_file,
        });
    }
    Ok(Options::Run(config))
}

/// Read a secret file, stripping at most one trailing line ending. An empty or
/// oversized file is an error rather than silently disabling authentication.
fn read_password_file(path: &str, option: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|error| format!("{option} could not open password file: {error}"))?;
    let mut bytes = Vec::new();
    // One extra byte distinguishes oversized content, allowing a final CRLF.
    file.take(1027)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{option} could not read password file: {error}"))?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    if bytes.is_empty() {
        return Err(format!("{option} must contain a nonempty password"));
    }
    if bytes.len() > 1024 {
        return Err(format!("{option} must not exceed 1024 bytes"));
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

#[cfg(unix)]
struct ShutdownSignals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(windows)]
struct ShutdownSignals {
    ctrl_c: tokio::signal::windows::CtrlC,
}

#[cfg(not(any(unix, windows)))]
struct ShutdownSignals;

impl ShutdownSignals {
    fn install(runtime: &tokio::runtime::Runtime) -> io::Result<Self> {
        #[cfg(unix)]
        {
            runtime.block_on(async {
                Ok(Self {
                    interrupt: tokio::signal::unix::signal(
                        tokio::signal::unix::SignalKind::interrupt(),
                    )?,
                    terminate: tokio::signal::unix::signal(
                        tokio::signal::unix::SignalKind::terminate(),
                    )?,
                })
            })
        }
        #[cfg(windows)]
        {
            runtime.block_on(async {
                Ok(Self {
                    ctrl_c: tokio::signal::windows::ctrl_c()?,
                })
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = runtime;
            Ok(Self)
        }
    }

    fn poll(&mut self, context: &mut Context<'_>) -> Poll<&'static str> {
        #[cfg(unix)]
        {
            if self.interrupt.poll_recv(context).is_ready() {
                return Poll::Ready("SIGINT");
            }
            if self.terminate.poll_recv(context).is_ready() {
                return Poll::Ready("SIGTERM");
            }
        }
        #[cfg(windows)]
        if self.ctrl_c.poll_recv(context).is_ready() {
            return Poll::Ready("CTRL_C");
        }
        #[cfg(not(any(unix, windows)))]
        let _ = context;
        Poll::Pending
    }
}

enum ShutdownTrigger {
    Signal(&'static str),
    Cancelled,
}

enum ShutdownOutcome {
    Complete,
    Forced(&'static str),
}

fn wait_for_shutdown_trigger(
    runtime: &tokio::runtime::Runtime,
    signals: &mut ShutdownSignals,
    cancel: tokio::sync::oneshot::Receiver<()>,
) -> ShutdownTrigger {
    runtime.block_on(async {
        let mut cancel = cancel;
        std::future::poll_fn(|context| {
            if let Poll::Ready(signal) = signals.poll(context) {
                return Poll::Ready(ShutdownTrigger::Signal(signal));
            }
            if Pin::new(&mut cancel).poll(context).is_ready() {
                return Poll::Ready(ShutdownTrigger::Cancelled);
            }
            Poll::Pending
        })
        .await
    })
}

fn wait_for_shutdown_completion(
    runtime: &tokio::runtime::Runtime,
    signals: &mut ShutdownSignals,
    completion: tokio::sync::oneshot::Receiver<()>,
    timeout: Duration,
) -> ShutdownOutcome {
    runtime.block_on(async {
        let mut deadline = std::pin::pin!(tokio::time::sleep(timeout));
        let mut completion = completion;
        std::future::poll_fn(|context| {
            if Pin::new(&mut completion).poll(context).is_ready() {
                return Poll::Ready(ShutdownOutcome::Complete);
            }
            if let Poll::Ready(signal) = signals.poll(context) {
                return Poll::Ready(ShutdownOutcome::Forced(signal));
            }
            if deadline.as_mut().poll(context).is_ready() {
                return Poll::Ready(ShutdownOutcome::Forced("shutdown deadline exceeded"));
            }
            Poll::Pending
        })
        .await
    })
}

fn supervise_shutdown(
    stopping: Arc<AtomicBool>,
    cancel: tokio::sync::oneshot::Receiver<()>,
    completion: tokio::sync::oneshot::Receiver<()>,
    ready: mpsc::SyncSender<io::Result<()>>,
    shutdown_deadline: Duration,
) -> io::Result<Option<&'static str>> {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(io::Error::new(error.kind(), error.to_string())));
            return Err(error);
        }
    };
    let mut signals = match ShutdownSignals::install(&runtime) {
        Ok(signals) => signals,
        Err(error) => {
            let _ = ready.send(Err(io::Error::new(error.kind(), error.to_string())));
            return Err(error);
        }
    };
    if ready.send(Ok(())).is_err() {
        return Ok(None);
    }

    match wait_for_shutdown_trigger(&runtime, &mut signals, cancel) {
        ShutdownTrigger::Cancelled => Ok(None),
        ShutdownTrigger::Signal(signal) => {
            eprintln!("rillcache: received {signal}; stopping admission and joining workers");
            stopping.store(true, Ordering::Release);
            match wait_for_shutdown_completion(
                &runtime,
                &mut signals,
                completion,
                shutdown_deadline,
            ) {
                ShutdownOutcome::Complete => Ok(Some(signal)),
                ShutdownOutcome::Forced(reason) => {
                    eprintln!("rillcache: forced termination: {reason}");
                    std::process::exit(1)
                }
            }
        }
    }
}

fn run_server(config: Config) -> io::Result<()> {
    let shutdown_deadline = config.shutdown_deadline;
    let server = Server::bind(config)?;
    let local_addr = server.local_addr()?;
    let tls_addr = server.tls_local_addr()?;
    let stopping = Arc::new(AtomicBool::new(false));
    let supervisor_stopping = Arc::clone(&stopping);
    let (cancel_sender, cancel_receiver) = tokio::sync::oneshot::channel();
    let (completion_sender, completion_receiver) = tokio::sync::oneshot::channel();
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let supervisor = thread::Builder::new()
        .name("rillcache-signal-supervisor".into())
        .spawn(move || {
            supervise_shutdown(
                supervisor_stopping,
                cancel_receiver,
                completion_receiver,
                ready_sender,
                shutdown_deadline,
            )
        })?;

    match ready_receiver.recv() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = supervisor.join();
            return Err(error);
        }
        Err(_) => {
            let _ = supervisor.join();
            return Err(io::Error::other("signal supervisor exited during startup"));
        }
    }

    eprintln!(
        "rillcache {} listening on {local_addr}",
        env!("CARGO_PKG_VERSION")
    );
    if let Some(addr) = tls_addr {
        eprintln!("rillcache TLS 1.3 listening on {addr}");
    }
    let server_result = server.run_until(stopping);
    let _ = completion_sender.send(());
    let _ = cancel_sender.send(());
    let signal = supervisor
        .join()
        .map_err(|_| io::Error::other("signal supervisor panicked"))??;
    if server_result.is_ok() && signal.is_some() {
        eprintln!(
            "rillcache: graceful shutdown completed; workers joined and enabled journal flushed"
        );
    }
    server_result
}

fn check_config(arguments: impl IntoIterator<Item = String>) -> ExitCode {
    // Parser diagnostics may contain user-controlled arguments. Do not echo them
    // in the preflight output, which is intended for retained deployment logs.
    let Ok(Options::Run(config)) = options(arguments) else {
        eprintln!("rillcache: configuration check requires valid startup options");
        return ExitCode::from(2);
    };
    if let Err(error) = config.validate() {
        eprintln!("rillcache: configuration check failed ({:?})", error.kind());
        return ExitCode::from(2);
    }
    println!("rillcache: startup options validated (server not started)");
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1).peekable();
    if arguments
        .next_if(|argument| argument == "--check-config")
        .is_some()
    {
        return check_config(arguments);
    }
    match options(arguments) {
        Ok(Options::Help) => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Options::Version) => {
            println!("rillcache {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Options::Healthcheck {
            address,
            username,
            password,
            require_aof,
            aof_write_probe_directory,
            min_payload_headroom,
            tls,
        }) => {
            let status = healthcheck::StatusChecks {
                require_aof,
                aof_write_probe_directory: aof_write_probe_directory.as_deref(),
                min_payload_headroom,
            };
            match healthcheck::check_with_status(
                address,
                username.as_deref(),
                password.as_deref(),
                status,
                tls,
            ) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("rillcache: health check failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Ok(Options::Run(config)) => match run_server(config) {
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
    fn shutdown_deadline_reports_forced_termination_if_join_has_not_finished() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut signals = ShutdownSignals::install(&runtime).unwrap();
        let (_sender, completion) = tokio::sync::oneshot::channel();
        assert!(matches!(
            wait_for_shutdown_completion(
                &runtime,
                &mut signals,
                completion,
                Duration::from_millis(1),
            ),
            ShutdownOutcome::Forced("shutdown deadline exceeded")
        ));
    }

    #[test]
    fn tls_client_certificate_mapping_parses_sha256_and_rejects_invalid_values() {
        let (fingerprint, username) = parse_tls_client_cert_user(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef=reader",
        )
        .unwrap();
        assert_eq!(fingerprint[0], 0x01);
        assert_eq!(fingerprint[31], 0xef);
        assert_eq!(username, b"reader");
        assert!(parse_tls_client_cert_user("abc=reader").is_err());
        assert!(parse_tls_client_cert_user(&format!("{}=reader", "gg".repeat(32))).is_err());
        assert!(parse_tls_client_cert_user(&format!("{}=bad user", "00".repeat(32))).is_err());
    }

    #[test]
    fn tls_client_certificate_mapping_file_option_is_parsed() {
        let Options::Run(config) = options(
            [
                "--tls-client-cert-users-file",
                "/etc/rillcache/client-users.map",
            ]
            .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected TLS mapping config");
        };
        assert_eq!(
            config.tls_client_cert_users_file.as_deref(),
            Some(std::path::Path::new("/etc/rillcache/client-users.map"))
        );
    }

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
                "--tls-ocsp-file",
                "server.ocsp.der",
                "--tls-client-ca-file",
                "ca.pem",
                "--tls-client-crl-file",
                "clients.crl.der",
                "--tls-client-cert-user",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa=cache-reader",
                "--tls-reload-trigger",
                "/run/rillcache/tls-generation",
                "--tls-sni-cert-file",
                "alt.test=alternate.pem",
                "--tls-sni-key-file",
                "alt.test=alternate.key",
                "--tls-sni-ocsp-file",
                "alt.test=alternate.ocsp.der",
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
            config.tls_ocsp_file.as_deref(),
            Some(std::path::Path::new("server.ocsp.der"))
        );
        assert_eq!(
            config.tls_client_ca_file.as_deref(),
            Some(std::path::Path::new("ca.pem"))
        );
        assert_eq!(
            config.tls_client_crl_file.as_deref(),
            Some(std::path::Path::new("clients.crl.der"))
        );
        assert_eq!(
            config
                .tls_client_cert_users
                .get(&[0xaa; 32])
                .map(Vec::as_slice),
            Some(b"cache-reader".as_slice())
        );
        assert_eq!(
            config.tls_reload_trigger.as_deref(),
            Some(std::path::Path::new("/run/rillcache/tls-generation"))
        );
        assert_eq!(config.tls_sni_certificates.len(), 1);
        assert_eq!(config.tls_sni_certificates[0].server_name, "alt.test");
        assert_eq!(
            config.tls_sni_certificates[0].cert_file,
            std::path::Path::new("alternate.pem")
        );
        assert_eq!(
            config.tls_sni_certificates[0].key_file,
            std::path::Path::new("alternate.key")
        );
        assert_eq!(
            config.tls_sni_certificates[0].ocsp_file.as_deref(),
            Some(std::path::Path::new("alternate.ocsp.der"))
        );
        assert_eq!(config.tls_handshake_timeout, Duration::from_millis(2500));
        assert!(
            options(["--tls-bind", "127.0.0.1:6379"].map(str::to_owned)).is_err(),
            "TLS listener must differ from the plaintext listener"
        );
    }

    #[test]
    fn tls_and_plain_cli_listeners_can_both_request_ephemeral_ports() {
        let Options::Run(config) = options(
            [
                "--bind",
                "127.0.0.1:0",
                "--tls-bind",
                "127.0.0.1:0",
                "--tls-cert-file",
                "server.pem",
                "--tls-key-file",
                "server.key",
            ]
            .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected ephemeral listener config");
        };
        assert_eq!(config.bind_addr.port(), 0);
        assert_eq!(config.tls_bind_addr, Some(config.bind_addr));
        config.validate().unwrap();
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
        std::fs::remove_file(path).unwrap();
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
        let Options::Run(authenticated_journal) = options(
            [
                "--appendonly",
                "yes",
                "--aof-hmac-key-file",
                "/run/secrets/aof.key",
                "--aof-hmac-migrate",
            ]
            .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected authenticated journal config");
        };
        assert_eq!(
            authenticated_journal.aof_hmac_key_file.as_deref(),
            Some(std::path::Path::new("/run/secrets/aof.key"))
        );
        assert!(authenticated_journal.aof_hmac_migrate);
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
        let Options::Healthcheck {
            address,
            username,
            password,
            require_aof,
            aof_write_probe_directory,
            min_payload_headroom,
            tls,
        } = options(["--bind", "0.0.0.0:6379", "--healthcheck", "[::1]:6379"].map(str::to_owned))
            .unwrap()
        else {
            panic!("expected health check");
        };
        assert_eq!(address, "[::1]:6379".parse().unwrap());
        assert!(password.is_none());
        assert!(username.is_none());
        assert!(!require_aof);
        assert!(aof_write_probe_directory.is_none());
        assert!(min_payload_headroom.is_none());
        assert!(tls.is_none());
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
    fn config_files_are_ordered_bounded_and_overridden_by_later_cli() {
        let path = std::env::temp_dir().join(format!(
            "rillcache-config-{}-{}.conf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            "# local startup\nbind 127.0.0.1:26379\nmaxclients 12\nauth-global-limit 20\nrequirepass \"file secret\"\n",
        )
        .unwrap();
        let path = path.to_str().unwrap();

        let Options::Run(from_file) = options(["--config", path].map(str::to_owned)).unwrap()
        else {
            panic!("expected config file");
        };
        assert_eq!(from_file.bind_addr, "127.0.0.1:26379".parse().unwrap());
        assert_eq!(from_file.max_clients, 12);
        assert_eq!(from_file.auth_global_limit, 20);
        assert_eq!(
            from_file.requirepass.as_deref(),
            Some(b"file secret".as_slice())
        );

        let Options::Run(overridden) = options(
            [
                "--config",
                path,
                "--max-clients",
                "34",
                "--requirepass",
                "cli secret",
            ]
            .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected override");
        };
        assert_eq!(overridden.max_clients, 34);
        assert_eq!(
            overridden.requirepass.as_deref(),
            Some(b"cli secret".as_slice())
        );

        let Options::Run(later_file) =
            options(["--max-clients", "34", "--config", path].map(str::to_owned)).unwrap()
        else {
            panic!("expected later config");
        };
        assert_eq!(later_file.max_clients, 12);

        assert!(options(["--config"].map(str::to_owned)).is_err());
        assert!(options(["--config", ""].map(str::to_owned)).is_err());
        assert!(
            options(["--config", "/definitely/missing/rillcache.conf"].map(str::to_owned)).is_err()
        );

        std::fs::write(path, "unknown-directive value\n").unwrap();
        assert!(options(["--config", path].map(str::to_owned)).is_err());

        std::fs::write(path, "").unwrap();
        assert!(options(["--config", path].map(str::to_owned)).is_err());
        std::fs::remove_file(path).unwrap();
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
            vec!["--auth-global-limit", "10001"],
            vec!["--auth-global-limit", "-1"],
            vec!["--auth-global-limit", "x"],
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
            vec!["--frame-assembly-timeout-ms", "-1"],
            vec!["--frame-assembly-timeout-ms", "600001"],
            vec!["--frame-assembly-timeout-ms", "nonsense"],
            vec!["--healthcheck"],
            vec!["--healthcheck", "localhost:6379"],
            vec!["--healthcheck", "127.0.0.1:0"],
            vec!["--healthcheck", "127.0.0.1:6379", "--unknown"],
            vec!["--healthcheck-require-aof"],
            vec!["--healthcheck-tls-ca-file", "ignored"],
            vec!["--healthcheck-server-name", "localhost"],
            vec![
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-tls-ca-file",
            ],
            vec![
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-server-name",
            ],
            vec![
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-server-name",
                "localhost",
            ],
            vec![
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-tls-ca-file",
                "ignored",
                "--healthcheck-tls-ca-file",
                "ignored",
            ],
            vec![
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-server-name",
                "localhost",
                "--healthcheck-server-name",
                "localhost",
            ],
            vec![
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-require-aof",
                "--healthcheck-require-aof",
            ],
            vec!["--healthcheck-password-file", "password"],
            vec![
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-password-file",
            ],
            vec![
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-password-file",
                "/definitely/missing/probe-password",
            ],
            vec!["--requirepass"],
            vec!["--socket-workers", "0"],
            vec!["--socket-workers", "17"],
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
                "--auth-global-limit",
                "20",
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
                "--frame-assembly-timeout-ms",
                "0",
                "--write-stall-timeout-ms",
                "750",
                "--global-command-budget",
                "500",
                "--user-command-budget",
                "50",
                "--socket-workers",
                "3",
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
        assert_eq!(config.auth_global_limit, 20);
        assert_eq!(config.auth_source_window_ms, 250);
        assert_eq!(config.worker_threads, Some(4));
        assert_eq!(config.socket_workers, Some(3));
        assert_eq!(config.max_data_bytes, 100);
        assert_eq!(config.limits.max_bulk_bytes, 8192);
        assert_eq!(config.limits.max_frame_bytes, 16384);
        assert_eq!(config.idle_timeout, Duration::from_millis(250));
        assert_eq!(config.frame_assembly_timeout, None);
        assert_eq!(config.write_stall_timeout, Some(Duration::from_millis(750)));
        assert_eq!(config.global_command_budget, 500);
        assert_eq!(config.user_command_budget, 50);
        assert_eq!(
            Config::default().frame_assembly_timeout,
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            Config::default().write_stall_timeout,
            Some(Duration::from_secs(30))
        );
        assert_eq!(Config::default().global_command_budget, 0);
        assert_eq!(Config::default().user_command_budget, 0);
    }

    #[test]
    fn overload_policy_options_reject_out_of_range_values() {
        let Options::Run(disabled) =
            options(["--write-stall-timeout-ms", "0"].map(str::to_owned)).unwrap()
        else {
            panic!("expected configuration");
        };
        assert_eq!(disabled.write_stall_timeout, None);
        for arguments in [
            vec!["--write-stall-timeout-ms"],
            vec!["--write-stall-timeout-ms", "-1"],
            vec!["--write-stall-timeout-ms", "600001"],
            vec!["--write-stall-timeout-ms", "nonsense"],
            vec!["--global-command-budget"],
            vec!["--global-command-budget", "1000001"],
            vec!["--global-command-budget", "-1"],
            vec!["--user-command-budget", "nonsense"],
            vec!["--user-command-budget", "1000001"],
        ] {
            assert!(
                options(arguments.clone().into_iter().map(str::to_owned)).is_err(),
                "{arguments:?} must be rejected"
            );
        }
    }

    #[test]
    fn healthcheck_aof_write_probe_requires_reported_journal_check() {
        let directory = std::path::Path::new("/var/lib/rillcache");
        let Options::Healthcheck {
            require_aof,
            aof_write_probe_directory,
            ..
        } = options(
            [
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-aof-write-probe",
                directory.to_str().unwrap(),
                "--healthcheck-require-aof",
            ]
            .map(str::to_owned),
        )
        .unwrap()
        else {
            panic!("expected healthcheck");
        };
        assert!(require_aof);
        assert_eq!(aof_write_probe_directory.as_deref(), Some(directory));
        for arguments in [
            vec![
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-aof-write-probe",
                "/tmp",
            ],
            vec![
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-require-aof",
                "--healthcheck-aof-write-probe",
                "",
            ],
            vec![
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-require-aof",
                "--healthcheck-aof-write-probe",
                "/tmp",
                "--healthcheck-aof-write-probe",
                "/tmp",
            ],
        ] {
            assert!(options(arguments.into_iter().map(str::to_owned)).is_err());
        }
    }

    #[test]
    fn healthcheck_tls_name_is_separate_from_the_numeric_socket_address() {
        let ca = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tls/ca.crt");
        let Options::Healthcheck { address, tls, .. } = options(
            [
                "--healthcheck",
                "127.0.0.1:6379",
                "--healthcheck-server-name",
                "localhost",
                "--healthcheck-tls-ca-file",
                ca.to_str().unwrap(),
            ]
            .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected healthcheck");
        };
        assert_eq!(address, "127.0.0.1:6379".parse().unwrap());
        assert!(tls.is_some());
    }

    #[test]
    fn password_files_enforce_the_payload_limit_and_strip_only_one_line_ending() {
        let path = std::env::temp_dir().join(format!(
            "rillcache-password-bound-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let name = path.to_str().unwrap();
        for ending in [b"".as_slice(), b"\n", b"\r\n"] {
            let expected = vec![b'x'; 1024];
            std::fs::write(&path, [expected.as_slice(), ending].concat()).unwrap();
            assert_eq!(
                read_password_file(name, "--healthcheck-password-file").unwrap(),
                expected
            );
        }
        std::fs::write(&path, b"binary\0\xff\n\n").unwrap();
        assert_eq!(
            read_password_file(name, "--healthcheck-password-file").unwrap(),
            b"binary\0\xff\n"
        );
        for content in [
            Vec::new(),
            b"\r\n".to_vec(),
            vec![b'x'; 1025],
            vec![b'x'; 16384],
        ] {
            std::fs::write(&path, content).unwrap();
            assert!(read_password_file(name, "--healthcheck-password-file").is_err());
            assert!(read_password_file(name, "--requirepass-file").is_err());
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn secure_secret_files_flag_and_password_path_parse() {
        let path = std::env::temp_dir().join(format!(
            "rillcache-secure-flag-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, b"password\n").unwrap();
        let Options::Run(config) = options(
            [
                "--secure-secret-files",
                "--requirepass-file",
                path.to_str().unwrap(),
            ]
            .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected run config");
        };
        assert!(config.secure_secret_files);
        assert_eq!(config.requirepass_file.as_deref(), Some(path.as_path()));
        assert_eq!(config.requirepass.as_deref(), Some(b"password".as_slice()));
        assert!(!format!("{config:?}").contains(path.to_str().unwrap()));
        let _ = std::fs::remove_file(path);
    }
}
