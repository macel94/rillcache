#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::time::Duration;

use rillcache::{Config, Server};

const HELP: &str = "rillcache — safe Rust, standard library only\n\nUsage: rillcache [OPTIONS]\n\n  --bind ADDRESS           Numeric socket address (default 127.0.0.1:6379)\n  --max-clients N          Maximum simultaneous clients (default 128)\n  --threads N              Fixed I/O/execution workers, 1..64 (default: thread per client)\n  --max-data-bytes N       Key/value payload budget (default 67108864)\n  --idle-timeout-ms N      Socket inactivity timeout (default 30000)\n  --help                  Show this help\n  --version               Show version\n\nExperimental: no authentication, TLS, persistence or eviction. Keep on loopback.";

enum Options {
    Run(Config),
    Help,
    Version,
}

fn options(arguments: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut config = Config::default();
    let mut arguments = arguments.into_iter();
    while let Some(option) = arguments.next() {
        match option.as_str() {
            "--help" | "-h" => return Ok(Options::Help),
            "--version" | "-V" => return Ok(Options::Version),
            "--bind" | "--max-clients" | "--threads" | "--max-data-bytes" | "--idle-timeout-ms" => {
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
                    "--max-clients" => config.max_clients = positive(&option, &value)?,
                    "--max-data-bytes" => config.max_data_bytes = positive(&option, &value)?,
                    "--idle-timeout-ms" => {
                        config.idle_timeout = Duration::from_millis(
                            value
                                .parse::<u64>()
                                .ok()
                                .filter(|&n| n > 0)
                                .ok_or("--idle-timeout-ms requires a positive integer")?,
                        )
                    }
                    _ => unreachable!(),
                }
            }
            _ => return Err(format!("unknown option: {option}")),
        }
    }
    Ok(Options::Run(config))
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
        Ok(Options::Run(config)) => match Server::bind(config).and_then(|server| {
            eprintln!(
                "rillcache {} listening on {}",
                env!("CARGO_PKG_VERSION"),
                server.local_addr()?
            );
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
    fn cli_validates_arguments() {
        for arguments in [
            vec!["--unknown"],
            vec!["--bind"],
            vec!["--bind", "localhost:6379"],
            vec!["--max-clients", "0"],
            vec!["--threads", "0"],
            vec!["--threads", "65"],
            vec!["--threads", "-1"],
            vec!["--max-data-bytes", "-1"],
            vec!["--idle-timeout-ms", "0"],
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
                "--max-data-bytes",
                "100",
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
        assert_eq!(config.worker_threads, Some(4));
        assert_eq!(config.max_data_bytes, 100);
        assert_eq!(config.idle_timeout, Duration::from_millis(250));
    }
}
