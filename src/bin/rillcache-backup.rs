#![forbid(unsafe_code)]

//! Offline, integrity-checked backup and restore for the private RESP AOF.
//!
//! The journal's exclusive ownership lock is used to reject a live server during
//! backup. Restore validates the archive and replays the journal in an isolated
//! temporary path before atomically publishing it at the destination.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rillcache::engine::{Auth, FsyncPolicy, JournalAuth, Store, read_auth_key_file};
use rillcache::protocol::Limits;

const MAGIC: &[u8] = b"RILLCACHE-AOF-BACKUP-V1\n";
const CHUNK: usize = 1024 * 1024;
const DEFAULT_MAX_DATA_BYTES: usize = 64 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 16 * 1024 * 1024 * 1024;

const HELP: &str = "rillcache-backup — offline AOF backup and restore\n\nUsage:\n  rillcache-backup create --dir DIR [--filename NAME] --output FILE [LIMITS]\n  rillcache-backup verify --input FILE [LIMITS]\n  rillcache-backup restore --input FILE --dir DIR [--filename NAME] [LIMITS]\n\nLIMITS: --max-data-bytes N --max-frame-bytes N --max-bulk-bytes N --max-arguments N\n       --aof-hmac-key-file PATH (authenticated journals)\n\nThe versioned archive stores a RESP AOF byte stream with its length and CRC32.\n`create` requires the source server to be stopped (the AOF lock enforces this).\n`restore` validates the archive and replays it in a temporary directory, then\ninstalls it only if the destination journal does not already exist. This is an\noffline, quiescent backup; it is not online backup or point-in-time recovery.\nCRC32 detects accidental corruption, not malicious modification.\n";

#[derive(Debug)]
struct Options {
    action: String,
    dir: Option<PathBuf>,
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    filename: String,
    max_data_bytes: usize,
    limits: Limits,
    aof_hmac_key_file: Option<PathBuf>,
}

fn parse_args() -> Result<Option<Options>, String> {
    let mut args = env::args_os().skip(1);
    let Some(action) = args.next() else {
        return Ok(None);
    };
    if action == "--help" || action == "-h" || action == "help" {
        println!("{HELP}");
        return Ok(None);
    }
    if action == "--version" || action == "version" {
        println!("rillcache-backup {}", env!("CARGO_PKG_VERSION"));
        return Ok(None);
    }
    let action = action
        .into_string()
        .map_err(|_| "action is not UTF-8".to_owned())?;
    if !matches!(action.as_str(), "create" | "verify" | "restore") {
        return Err(format!(
            "unknown action {action:?}; use create, verify or restore"
        ));
    }
    let mut options = Options {
        action,
        dir: None,
        input: None,
        output: None,
        filename: "appendonly.aof".into(),
        max_data_bytes: DEFAULT_MAX_DATA_BYTES,
        limits: Limits::default(),
        aof_hmac_key_file: None,
    };
    while let Some(argument) = args.next() {
        let key = argument
            .into_string()
            .map_err(|_| "option is not UTF-8".to_owned())?;
        let value = args
            .next()
            .ok_or_else(|| format!("{key} requires a value"))?;
        match key.as_str() {
            "--dir" => options.dir = Some(PathBuf::from(value)),
            "--input" => options.input = Some(PathBuf::from(value)),
            "--output" => options.output = Some(PathBuf::from(value)),
            "--aof-hmac-key-file" => {
                if value.is_empty() {
                    return Err("--aof-hmac-key-file requires a nonempty path".into());
                }
                options.aof_hmac_key_file = Some(PathBuf::from(value));
            }
            "--filename" => {
                options.filename = value
                    .into_string()
                    .map_err(|_| "filename is not UTF-8".to_owned())?;
            }
            "--max-data-bytes" | "--max-frame-bytes" | "--max-bulk-bytes" | "--max-arguments" => {
                let raw = value
                    .into_string()
                    .map_err(|_| format!("{key} is not UTF-8"))?;
                let parsed = raw
                    .parse::<usize>()
                    .map_err(|_| format!("{key} must be a positive integer"))?;
                match key.as_str() {
                    "--max-data-bytes" => options.max_data_bytes = parsed,
                    "--max-frame-bytes" => options.limits.max_frame_bytes = parsed,
                    "--max-bulk-bytes" => options.limits.max_bulk_bytes = parsed,
                    "--max-arguments" => options.limits.max_arguments = parsed,
                    _ => unreachable!(),
                }
            }
            _ => return Err(format!("unknown option {key:?}")),
        }
    }
    if options.max_data_bytes == 0
        || options.limits.max_frame_bytes < 8
        || options.limits.max_bulk_bytes == 0
        || options.limits.max_bulk_bytes > options.limits.max_frame_bytes
        || options.limits.max_arguments == 0
    {
        return Err("limits must be positive; max-frame-bytes must be at least 8 and max-bulk-bytes cannot exceed it".into());
    }
    validate_filename(&options.filename)?;
    match options.action.as_str() {
        "create" if options.dir.is_none() || options.output.is_none() => {
            return Err("create requires --dir and --output".into());
        }
        "verify" if options.input.is_none() => return Err("verify requires --input".into()),
        "restore" if options.input.is_none() || options.dir.is_none() => {
            return Err("restore requires --input and --dir".into());
        }
        _ => {}
    }
    Ok(Some(options))
}

fn validate_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty() || filename == "." || filename == ".." || filename.contains(['/', '\\'])
    {
        return Err("filename must be a single basename".into());
    }
    Ok(())
}

fn lock_path(path: &Path) -> PathBuf {
    path.with_extension(format!(
        "{}lock",
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| format!("{extension}."))
            .unwrap_or_default()
    ))
}

fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    crc
}

fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

struct TempPath(PathBuf);

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn create_temp(destination: &Path, label: &str) -> io::Result<(TempPath, File)> {
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "destination parent directory must already exist",
        ));
    }
    let name = destination
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing destination filename"))?
        .to_string_lossy();
    let temporary = parent.join(format!(".{name}.{label}-{}.partial", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    Ok((TempPath(temporary.clone()), options.open(temporary)?))
}

/// Atomically publishes a complete file without replacing an existing path.
fn publish_noreplace(temporary: &TempPath, destination: &Path) -> io::Result<()> {
    // hard_link is an atomic create-if-absent on the same filesystem; rename
    // would silently overwrite an existing backup/destination.
    fs::hard_link(&temporary.0, destination)?;
    sync_parent(destination)
}

fn copy_crc<R: Read, W: Write + ?Sized>(reader: &mut R, writer: &mut W) -> io::Result<(u64, u32)> {
    let mut buffer = vec![0u8; CHUNK];
    let mut length = 0u64;
    let mut crc = !0u32;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let next_length = length
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("backup length overflow"))?;
        if next_length > MAX_ARCHIVE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "journal exceeds the 16 GiB archive limit",
            ));
        }
        writer.write_all(&buffer[..read])?;
        length = next_length;
        crc = crc32_update(crc, &buffer[..read]);
    }
    Ok((length, !crc))
}

fn journal_path(directory: &Path, filename: &str) -> PathBuf {
    directory.join(filename)
}

fn canonical_target(path: &Path) -> io::Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing path basename"))?;
    Ok(fs::canonicalize(parent)?.join(name))
}

fn journal_auth(options: &Options) -> io::Result<Option<JournalAuth>> {
    options
        .aof_hmac_key_file
        .as_deref()
        .map(|path| read_auth_key_file(path).and_then(|key| JournalAuth::new(key, false)))
        .transpose()
}

fn validate_journal(
    path: &Path,
    max_data_bytes: usize,
    limits: Limits,
    auth: Option<JournalAuth>,
) -> io::Result<()> {
    let _store = Store::with_auth_and_journal(
        max_data_bytes,
        Auth::default(),
        path,
        FsyncPolicy::Always,
        limits,
        auth,
    )?;
    // The store constructor has already replayed the log under the same exclusive
    // ownership lock. A forced sync guarantees the backup source is durable.
    _store.journal_flush()
}

fn create_backup(options: &Options) -> io::Result<()> {
    let directory = options.dir.as_deref().expect("validated --dir");
    let output = options.output.as_deref().expect("validated --output");
    let journal = journal_path(directory, &options.filename);
    let metadata = fs::symlink_metadata(&journal)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source journal must be a regular non-symlink file",
        ));
    }
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "journal exceeds the 16 GiB archive limit",
        ));
    }
    if fs::symlink_metadata(output).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "backup output already exists; refusing to overwrite",
        ));
    }
    let canonical_output = canonical_target(output)?;
    if canonical_target(&journal)? == canonical_output
        || canonical_target(&lock_path(&journal))? == canonical_output
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "backup output cannot be the live journal or its ownership lock path",
        ));
    }
    // Acquiring the normal journal lock rejects a live server and validates the
    // entire AOF before any archive is published.
    let store = Store::with_auth_and_journal(
        options.max_data_bytes,
        Auth::default(),
        &journal,
        FsyncPolicy::Always,
        options.limits,
        journal_auth(options)?,
    )?;
    store.journal_flush()?;
    let mut source = File::open(&journal)?;
    let (temporary, mut archive) = create_temp(output, "backup")?;
    let result = (|| -> io::Result<()> {
        archive.write_all(MAGIC)?;
        archive.write_all(&0u64.to_le_bytes())?;
        archive.write_all(&0u32.to_le_bytes())?;
        let (length, crc) = copy_crc(&mut source, &mut archive)?;
        if length > MAX_ARCHIVE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "journal exceeds the 16 GiB archive limit",
            ));
        }
        archive.seek(SeekFrom::Start(MAGIC.len() as u64))?;
        archive.write_all(&length.to_le_bytes())?;
        archive.write_all(&crc.to_le_bytes())?;
        archive.flush()?;
        archive.sync_all()?;
        drop(archive);
        publish_noreplace(&temporary, output)
    })();
    drop(store);
    result
}

fn copy_archive_payload<R: Read, W: Write + ?Sized>(
    reader: &mut R,
    writer: &mut W,
) -> io::Result<(u64, u32)> {
    copy_crc(reader, writer)
}

fn validate_archive(
    path: &Path,
    output_journal: Option<&Path>,
    max_data_bytes: usize,
    limits: Limits,
    auth: Option<JournalAuth>,
) -> io::Result<u64> {
    let mut archive = File::open(path)?;
    let mut magic = vec![0u8; MAGIC.len()];
    archive.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Rillcache AOF backup (magic/version differs)",
        ));
    }
    let mut u64_bytes = [0u8; 8];
    let mut u32_bytes = [0u8; 4];
    archive.read_exact(&mut u64_bytes)?;
    archive.read_exact(&mut u32_bytes)?;
    let expected_length = u64::from_le_bytes(u64_bytes);
    let expected_crc = u32::from_le_bytes(u32_bytes);
    if expected_length > MAX_ARCHIVE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "backup length outside supported bounds",
        ));
    }
    let stage_destination = if let Some(output) = output_journal {
        output.to_path_buf()
    } else {
        std::env::temp_dir().join(format!(
            "rillcache-backup-verify-{}-{}.aof",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    };
    let (temporary_path, mut temporary_file) = create_temp(&stage_destination, "restore")?;
    let mut limited = (&mut archive).take(expected_length);
    let (length, crc) = copy_archive_payload(&mut limited, &mut temporary_file)?;
    if length != expected_length || crc != expected_crc {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "backup length or CRC32 does not match",
        ));
    }
    let mut trailing = [0u8; 1];
    if archive.read(&mut trailing)? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "trailing bytes after backup payload",
        ));
    }
    temporary_file.flush()?;
    temporary_file.sync_all()?;
    drop(temporary_file);
    validate_journal(&temporary_path.0, max_data_bytes, limits, auth).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("backup journal failed replay validation: {error}"),
        )
    })?;
    if let Some(output) = output_journal {
        // Validation's Journal has been dropped, so its temporary lock is gone.
        if lock_path(&temporary_path.0).exists() {
            return Err(io::Error::other("temporary restore ownership lock remains"));
        }
        publish_noreplace(&temporary_path, output)?;
    }
    Ok(length)
}

fn restore_backup(options: &Options) -> io::Result<()> {
    let directory = options.dir.as_deref().expect("validated --dir");
    if !directory.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "restore directory must already exist",
        ));
    }
    let destination = journal_path(directory, &options.filename);
    if fs::symlink_metadata(&destination).is_ok() || lock_path(&destination).exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "destination journal or ownership lock already exists; refusing to overwrite",
        ));
    }
    validate_archive(
        options.input.as_deref().expect("validated --input"),
        Some(&destination),
        options.max_data_bytes,
        options.limits,
        journal_auth(options)?,
    )?;
    println!("restored validated AOF to {}", destination.display());
    Ok(())
}

fn verify_backup(options: &Options) -> io::Result<()> {
    let length = validate_archive(
        options.input.as_deref().expect("validated --input"),
        None,
        options.max_data_bytes,
        options.limits,
        journal_auth(options)?,
    )?;
    if options.aof_hmac_key_file.is_some() {
        println!(
            "verified Rillcache AOF backup: {length} journal bytes; CRC32, replay and HMAC-SHA256 authentication are valid"
        );
    } else {
        println!(
            "verified Rillcache AOF backup: {length} journal bytes; CRC32 and replay are valid (no authentication key supplied)"
        );
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let Some(options) = parse_args()? else {
        return Ok(());
    };
    let result = match options.action.as_str() {
        "create" => create_backup(&options),
        "verify" => verify_backup(&options),
        "restore" => restore_backup(&options),
        _ => unreachable!("action validated"),
    };
    result.map_err(|error| format!("{}: {error}", options.action))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rillcache-backup: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAGIC, crc32_update};

    #[test]
    fn backup_crc32_uses_the_standard_check_vector() {
        assert_eq!(!crc32_update(!0, b"123456789"), 0xcbf4_3926);
        assert_eq!(MAGIC, b"RILLCACHE-AOF-BACKUP-V1\n");
    }
}
