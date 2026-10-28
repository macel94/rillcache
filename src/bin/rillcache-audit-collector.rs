#![forbid(unsafe_code)]

//! Bounded UDP collector for `rillcache --audit-syslog` denial lines.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const HELP: &str = "rillcache-audit-collector — bounded UDP audit sink\n\nUsage: rillcache-audit-collector [OPTIONS]\n\n  --bind ADDRESS          UDP listen address (default 127.0.0.1:5514)\n  --output PATH           Append-only output file (default audit.log)\n  --max-bytes N           One-generation output bound (default 16777216)\n  --max-datagram N        Maximum accepted datagram bytes (default 4096)\n  --max-datagrams N       Exit after N accepted datagrams; 0 runs forever (default 0)\n  --print-address         Print the bound address to stdout (for tests/supervisors)\n  --help                  Show this help\n  --version               Show version\n\nUDP delivery is best-effort: datagrams can be lost, duplicated or reordered. The\ncollector stores signed lines but does not verify HMACs. Bind it to loopback or a\nprivate network and protect the output file with filesystem permissions.";

struct Options {
    bind: SocketAddr,
    output: PathBuf,
    max_bytes: u64,
    max_datagram: usize,
    max_datagrams: u64,
    print_address: bool,
}

fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        bind: "127.0.0.1:5514".parse().expect("static address"),
        output: PathBuf::from("audit.log"),
        max_bytes: 16 * 1024 * 1024,
        max_datagram: 4096,
        max_datagrams: 0,
        print_address: false,
    };
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--help" | "-h" => return Err(HELP.into()),
            "--version" | "-V" => {
                return Err(format!(
                    "rillcache-audit-collector {}",
                    env!("CARGO_PKG_VERSION")
                ));
            }
            "--print-address" => options.print_address = true,
            "--bind" => {
                options.bind = arguments
                    .next()
                    .ok_or("missing value for --bind")?
                    .parse()
                    .map_err(|_| "--bind requires a numeric IP:port address")?;
            }
            "--output" => {
                let value = arguments.next().ok_or("missing value for --output")?;
                if value.is_empty() {
                    return Err("--output requires a nonempty path".into());
                }
                options.output = PathBuf::from(value);
            }
            "--max-bytes" => {
                options.max_bytes = arguments
                    .next()
                    .ok_or("missing value for --max-bytes")?
                    .parse::<u64>()
                    .ok()
                    .filter(|bytes| (1024..=1024 * 1024 * 1024).contains(bytes))
                    .ok_or("--max-bytes requires 1024..=1073741824")?;
            }
            "--max-datagram" => {
                options.max_datagram = arguments
                    .next()
                    .ok_or("missing value for --max-datagram")?
                    .parse::<usize>()
                    .ok()
                    .filter(|bytes| (64..=65507).contains(bytes))
                    .ok_or("--max-datagram requires 64..=65507")?;
            }
            "--max-datagrams" => {
                options.max_datagrams = arguments
                    .next()
                    .ok_or("missing value for --max-datagrams")?
                    .parse::<u64>()
                    .map_err(|_| "--max-datagrams requires a nonnegative integer")?;
            }
            _ => return Err(format!("unknown argument {argument}")),
        }
    }
    Ok(options)
}

struct Output {
    file: File,
    path: PathBuf,
    rotated: PathBuf,
    max_bytes: u64,
    written: u64,
}

impl Output {
    fn open(path: &Path, max_bytes: u64) -> io::Result<Self> {
        let file = open_append(path)?;
        let written = file.metadata().map_or(0, |metadata| metadata.len());
        let mut rotated = OsString::from(path.as_os_str());
        rotated.push(".1");
        Ok(Self {
            file,
            path: path.to_path_buf(),
            rotated: PathBuf::from(rotated),
            max_bytes,
            written,
        })
    }

    fn append(&mut self, line: &[u8]) -> io::Result<()> {
        let mut line = line.to_vec();
        if !line.ends_with(b"\n") {
            line.push(b'\n');
        }
        if self.written > 0 && self.written.saturating_add(line.len() as u64) > self.max_bytes {
            self.rotate()?;
        }
        self.file.write_all(&line)?;
        self.file.flush()?;
        self.written = self.written.saturating_add(line.len() as u64);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        match fs::remove_file(&self.rotated) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(&self.path, &self.rotated)?;
        self.file = open_append(&self.path)?;
        self.written = 0;
        Ok(())
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn run(options: Options) -> io::Result<()> {
    let socket = UdpSocket::bind(options.bind)?;
    if options.print_address {
        println!("listening on {}", socket.local_addr()?);
    }
    let mut output = Output::open(&options.output, options.max_bytes)?;
    let mut buffer = vec![0u8; options.max_datagram.saturating_add(1)];
    let mut accepted = 0u64;
    let mut rejected = 0u64;
    loop {
        let (length, _) = match socket.recv_from(&mut buffer) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if length == 0 || length > options.max_datagram {
            rejected = rejected.saturating_add(1);
            continue;
        }
        output.append(&buffer[..length])?;
        accepted = accepted.saturating_add(1);
        if options.max_datagrams > 0 && accepted >= options.max_datagrams {
            break;
        }
    }
    eprintln!("audit collector accepted {accepted}, rejected {rejected}");
    Ok(())
}

fn main() -> ExitCode {
    let options = match parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return if message.starts_with("rillcache-audit-collector ") {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            };
        }
    };
    match run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("audit collector failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rillcache-audit-collector-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn append_adds_newlines_and_rotates_once() {
        let path = temp_path("rotate");
        let mut output = Output::open(&path, 1024).unwrap();
        for _ in 0..30 {
            output
                .append(b"0123456789012345678901234567890123456789")
                .unwrap();
        }
        assert!(output.written <= 1024);
        let mut rotated = path.as_os_str().to_os_string();
        rotated.push(".1");
        assert!(PathBuf::from(rotated).exists());
        let current = fs::read_to_string(&path).unwrap();
        assert!(current.ends_with('\n'));
        let _ = fs::remove_file(&path);
        let mut rotated = path.as_os_str().to_os_string();
        rotated.push(".1");
        let _ = fs::remove_file(PathBuf::from(rotated));
    }

    #[test]
    fn options_validate_bounds() {
        assert!(parse(["--output", ""].map(str::to_owned)).is_err());
        assert!(parse(["--max-bytes", "1023"].map(str::to_owned)).is_err());
        assert!(parse(["--max-datagram", "63"].map(str::to_owned)).is_err());
        assert!(parse(["--max-datagram", "65508"].map(str::to_owned)).is_err());
        let options = parse(
            [
                "--bind",
                "127.0.0.1:0",
                "--output",
                "/tmp/a.log",
                "--max-bytes",
                "4096",
                "--max-datagram",
                "512",
                "--max-datagrams",
                "3",
                "--print-address",
            ]
            .map(str::to_owned),
        )
        .unwrap();
        assert_eq!(options.max_bytes, 4096);
        assert_eq!(options.max_datagram, 512);
        assert_eq!(options.max_datagrams, 3);
        assert!(options.print_address);
    }
}
