//! Multi-user ACL rules with password digests and command/key permissions.
//!
//! The default user keeps the single-password behavior from S27. Additional users
//! add Redis-shaped `on/off`, password, command-category/command, key and channel
//! rules. Channel patterns are stored for future pub/sub but have nothing to
//! enforce until channels exist. This is not a complete Redis ACL implementation:
//! selectors, subcommand categories beyond explicit `cmd|sub` rules, ACL LOG, and
//! rate limiting are absent and documented in `docs/security.md`.

use super::{discovery, sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

const MAX_PASSWORD_BYTES: usize = 1024;
const MAX_RULE_BYTES: usize = 4096;
const MAX_ACL_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_USERS: usize = 10_000;
const MAX_RULES: usize = 1024;
const MAX_PATTERNS: usize = 1024;
const MAX_PATTERN_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Rule {
    All(bool),
    Category(Vec<u8>, bool),
    Command(Vec<u8>, Option<Vec<u8>>, bool),
}

#[derive(Clone, Debug)]
struct User {
    enabled: bool,
    nopass: bool,
    passwords: Vec<[u8; 32]>,
    rules: Vec<Rule>,
    keys: Vec<Vec<u8>>,
    channels: Vec<Vec<u8>>,
}

impl Default for User {
    /// A freshly created or reset user is off, cannot authenticate and has no
    /// command, key or channel permissions.
    fn default() -> Self {
        Self {
            enabled: false,
            nopass: false,
            passwords: Vec::new(),
            rules: Vec::new(),
            keys: Vec::new(),
            channels: Vec::new(),
        }
    }
}

impl User {
    fn allows_command(&self, arguments: &[&[u8]]) -> bool {
        let Some(command) = arguments.first() else {
            return false;
        };
        let subcommand = arguments.get(1);
        let mut allowed = false;
        for rule in &self.rules {
            let matches = match rule {
                Rule::All(_) => true,
                Rule::Category(category, _) => {
                    discovery::category_matches(command, arguments, category)
                }
                Rule::Command(name, sub, _) => {
                    command.eq_ignore_ascii_case(name)
                        && sub.as_ref().is_none_or(|expected| {
                            subcommand.is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
                        })
                }
            };
            if matches {
                allowed = match rule {
                    Rule::All(value) | Rule::Category(_, value) | Rule::Command(_, _, value) => {
                        *value
                    }
                };
            }
        }
        allowed
    }

    fn allows_key(&self, key: &[u8]) -> bool {
        self.keys
            .iter()
            .any(|pattern| super::keyspace::glob_match(pattern, key))
    }

    fn allows_channel(&self, channel: &[u8]) -> bool {
        self.channels
            .iter()
            .any(|pattern| super::keyspace::glob_match(pattern, channel))
    }

    fn rules_string(&self) -> String {
        if self.rules.is_empty() {
            return "-@all".into();
        }
        self.rules
            .iter()
            .map(|rule| match rule {
                Rule::All(true) => "+@all".to_string(),
                Rule::All(false) => "-@all".to_string(),
                Rule::Category(name, allow) => {
                    format!(
                        "{}@{}",
                        if *allow { "+" } else { "-" },
                        String::from_utf8_lossy(name)
                    )
                }
                Rule::Command(name, sub, allow) => {
                    let mut text = format!(
                        "{}{}",
                        if *allow { "+" } else { "-" },
                        String::from_utf8_lossy(name)
                    );
                    if let Some(sub) = sub {
                        text.push('|');
                        text.push_str(&String::from_utf8_lossy(sub));
                    }
                    text
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn keys_string(&self) -> String {
        self.keys
            .iter()
            .map(|pattern| format!("~{}", String::from_utf8_lossy(pattern)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn channels_string(&self) -> String {
        self.channels
            .iter()
            .map(|pattern| format!("&{}", String::from_utf8_lossy(pattern)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn password_string(&self) -> String {
        if self.nopass {
            return "nopass".into();
        }
        if self.passwords.is_empty() {
            return "resetpass".into();
        }
        self.passwords
            .iter()
            .map(|digest| {
                let mut text = String::from("#");
                for byte in digest {
                    text.push_str(&format!("{byte:02x}"));
                }
                text
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn acl_line(&self, name: &str) -> String {
        format!(
            "user {} {} {} {} {} {}",
            name,
            if self.enabled { "on" } else { "off" },
            self.password_string(),
            if self.keys.is_empty() {
                "resetkeys".to_string()
            } else {
                self.keys_string()
            },
            if self.channels.is_empty() {
                "resetchannels".to_string()
            } else {
                self.channels_string()
            },
            self.rules_string()
        )
    }
}

#[derive(Clone, Debug)]
pub struct Auth {
    users: BTreeMap<Vec<u8>, User>,
    aclfile: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserDetails {
    pub flags: Vec<&'static str>,
    pub passwords: Vec<String>,
    pub commands: String,
    pub keys: String,
    pub channels: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// No password is configured for the default user.
    NoPassword,
    /// Unknown/disabled user or wrong password.
    WrongPassword,
    /// A poisoned credential lock; fail closed rather than allow access.
    Internal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Allowed,
    NoAuth,
    DeniedCommand,
    DeniedKey,
    DeniedChannel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AclError {
    Syntax,
    UnknownRule,
    UnknownCommand,
    UnknownCategory,
    InvalidName,
    InvalidPassword,
    ProtectedDefault,
    NoAclFile,
    MissingDefault,
    TooLarge,
    Io(String),
}

impl Default for Auth {
    fn default() -> Self {
        Self::from_password(None)
    }
}

impl Auth {
    /// Default user with all commands and keys. An empty password explicitly
    /// disables authentication, matching `CONFIG SET requirepass ""`.
    pub fn from_password(password: Option<&[u8]>) -> Self {
        let mut users = BTreeMap::new();
        users.insert(b"default".to_vec(), Self::default_user(password));
        Self {
            users,
            aclfile: None,
        }
    }

    fn default_user(password: Option<&[u8]>) -> User {
        let mut user = User {
            enabled: true,
            nopass: true,
            rules: vec![Rule::All(true)],
            keys: vec![b"*".to_vec()],
            channels: vec![b"*".to_vec()],
            ..User::default()
        };
        if let Some(password) = password.filter(|password| !password.is_empty()) {
            user.nopass = false;
            user.passwords = vec![sha256::digest(password)];
        }
        user
    }

    pub fn from_acl_file(path: &Path) -> Result<Self, AclError> {
        let mut auth = Self {
            users: BTreeMap::new(),
            aclfile: Some(path.to_path_buf()),
        };
        auth.load_file(path)?;
        Ok(auth)
    }

    pub fn required(&self) -> bool {
        self.users
            .get(b"default".as_slice())
            .is_some_and(|user| user.enabled && !user.nopass)
    }

    pub fn configured(&self) -> bool {
        self.required()
    }

    /// No authentication, no extra users and no restrictive default user: every
    /// command can skip ACL lookup on the performance-sensitive default path.
    pub fn open(&self) -> bool {
        self.users.len() == 1
            && self.users.get(b"default".as_slice()).is_some_and(|user| {
                user.enabled
                    && user.nopass
                    && user.rules == [Rule::All(true)]
                    && user.keys == [b"*".to_vec()]
            })
    }

    pub fn set_password(&mut self, password: Option<&[u8]>) {
        let user = self
            .users
            .entry(b"default".to_vec())
            .or_insert_with(|| Self::default_user(None));
        if let Some(password) = password.filter(|password| !password.is_empty()) {
            user.enabled = true;
            user.nopass = false;
            user.passwords = vec![sha256::digest(password)];
        } else {
            user.nopass = true;
            user.passwords.clear();
        }
    }

    /// Verify credentials. `None` selects the default user. Named `nopass` users
    /// accept any password; the default user keeps the explicit no-password error
    /// used by Redis when only `requirepass` is relevant.
    pub fn verify(&self, username: Option<&[u8]>, password: &[u8]) -> Result<(), AuthError> {
        self.verify_user(username, password).map(|_| ())
    }

    /// Verify credentials and return the canonical stored user name so the
    /// session can retain a stable identity for `ACL WHOAMI` and later checks.
    pub fn verify_user(
        &self,
        username: Option<&[u8]>,
        password: &[u8],
    ) -> Result<Vec<u8>, AuthError> {
        let requested = username.unwrap_or(b"default");
        let Some(name) = self.lookup_name(requested) else {
            return Err(AuthError::WrongPassword);
        };
        let user = self.users.get(&name).expect("looked-up name exists");
        if !user.enabled {
            return Err(AuthError::WrongPassword);
        }
        if user.nopass {
            if name == b"default" {
                return Err(AuthError::NoPassword);
            }
            return Ok(name);
        }
        let digest = sha256::digest(password);
        if user
            .passwords
            .iter()
            .any(|stored| sha256::constant_time_eq(stored, &digest))
        {
            Ok(name)
        } else {
            Err(AuthError::WrongPassword)
        }
    }

    #[cfg(test)]
    fn lookup(&self, name: &[u8]) -> Option<&User> {
        self.users.get(name).or_else(|| {
            name.eq_ignore_ascii_case(b"default")
                .then(|| self.users.get(b"default".as_slice()))
                .flatten()
        })
    }

    fn lookup_name(&self, name: &[u8]) -> Option<Vec<u8>> {
        if let Some((key, _)) = self.users.get_key_value(name) {
            return Some(key.clone());
        }
        name.eq_ignore_ascii_case(b"default")
            .then(|| b"default".to_vec())
            .filter(|name| self.users.contains_key(name))
    }

    /// Resolve the effective user for this connection. An unauthenticated
    /// connection is the default user only while that user is enabled with
    /// `nopass`; otherwise `None` means `NOAUTH`.
    fn session_user(&self, authenticated: Option<&[u8]>) -> Option<(&[u8], &User)> {
        let name = match authenticated {
            Some(name) => self.lookup_name(name)?,
            None => b"default".to_vec(),
        };
        let user = self.users.get(&name)?;
        if !user.enabled || (authenticated.is_none() && !user.nopass) {
            return None;
        }
        Some((self.users.get_key_value(&name)?.0.as_slice(), user))
    }

    pub fn whoami(&self, authenticated: Option<&[u8]>) -> Option<Vec<u8>> {
        self.session_user(authenticated)
            .map(|(name, _)| name.to_vec())
    }

    pub fn authorize(&self, authenticated: Option<&[u8]>, arguments: &[&[u8]]) -> Access {
        let Some((_, user)) = self.session_user(authenticated) else {
            return Access::NoAuth;
        };
        // ACL WHOAMI is available to every authenticated user, as in Redis.
        if arguments
            .first()
            .is_some_and(|command| command.eq_ignore_ascii_case(b"ACL"))
            && arguments
                .get(1)
                .is_some_and(|subcommand| subcommand.eq_ignore_ascii_case(b"WHOAMI"))
        {
            return Access::Allowed;
        }
        if !user.allows_command(arguments) {
            return Access::DeniedCommand;
        }
        let Some(command) = arguments.first() else {
            return Access::Allowed;
        };
        for key in discovery::key_arguments(command, arguments) {
            if !user.allows_key(key) {
                return Access::DeniedKey;
            }
        }
        let channels: &[&[u8]] = if command.eq_ignore_ascii_case(b"PUBLISH") {
            &arguments[1..arguments.len().min(2)]
        } else if command.eq_ignore_ascii_case(b"SUBSCRIBE")
            || command.eq_ignore_ascii_case(b"UNSUBSCRIBE")
            || command.eq_ignore_ascii_case(b"PSUBSCRIBE")
            || command.eq_ignore_ascii_case(b"PUNSUBSCRIBE")
        {
            &arguments[1..]
        } else {
            &[]
        };
        for channel in channels {
            if !user.allows_channel(channel) {
                return Access::DeniedChannel;
            }
        }
        Access::Allowed
    }

    pub fn user_names(&self) -> Vec<Vec<u8>> {
        self.users.keys().cloned().collect()
    }

    pub fn acl_line(&self, name: &[u8]) -> Option<String> {
        self.users
            .get(name)
            .map(|user| user.acl_line(&String::from_utf8_lossy(name)))
    }

    pub fn details(&self, name: &[u8]) -> Option<UserDetails> {
        let user = self.users.get(name)?;
        let mut flags = vec![if user.enabled { "on" } else { "off" }];
        if user.nopass {
            flags.push("nopass");
        }
        if user.keys == [b"*".to_vec()] {
            flags.push("allkeys");
        }
        if user.channels == [b"*".to_vec()] {
            flags.push("allchannels");
        }
        Some(UserDetails {
            flags,
            passwords: user
                .passwords
                .iter()
                .map(|digest| {
                    let mut text = String::with_capacity(64);
                    for byte in digest {
                        text.push_str(&format!("{byte:02x}"));
                    }
                    text
                })
                .collect(),
            commands: user.rules_string(),
            keys: user.keys_string(),
            channels: user.channels_string(),
        })
    }

    pub(super) fn remove_user(&mut self, name: &[u8]) -> bool {
        self.users.remove(name).is_some()
    }

    pub fn setuser(&mut self, name: &[u8], rules: &[&[u8]]) -> Result<(), AclError> {
        if !valid_name(name) {
            return Err(AclError::InvalidName);
        }
        if rules.len() > MAX_RULES {
            return Err(AclError::TooLarge);
        }
        let mut users = self.users.clone();
        if !users.contains_key(name) && users.len() >= MAX_USERS {
            return Err(AclError::TooLarge);
        }
        let user = users.entry(name.to_vec()).or_default();
        for rule in rules {
            if rule.len() > MAX_RULE_BYTES || user.rules.len() >= MAX_RULES {
                return Err(AclError::TooLarge);
            }
            apply_rule(user, rule)?;
        }
        self.users = users;
        Ok(())
    }

    /// Load a complete ACL file into a replacement map before publishing it.
    pub fn load_file(&mut self, path: &Path) -> Result<(), AclError> {
        let metadata = fs::metadata(path).map_err(|error| AclError::Io(error.to_string()))?;
        if metadata.len() > MAX_ACL_FILE_BYTES {
            return Err(AclError::TooLarge);
        }
        let bytes = fs::read(path).map_err(|error| AclError::Io(error.to_string()))?;
        if bytes.len() as u64 > MAX_ACL_FILE_BYTES {
            return Err(AclError::TooLarge);
        }
        let text = String::from_utf8(bytes).map_err(|_| AclError::Syntax)?;
        let mut users: BTreeMap<Vec<u8>, User> = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut tokens = line.split_ascii_whitespace();
            if tokens.next() != Some("user") {
                return Err(AclError::Syntax);
            }
            let Some(name) = tokens.next() else {
                return Err(AclError::Syntax);
            };
            let name = name.as_bytes();
            if !valid_name(name) {
                return Err(AclError::InvalidName);
            }
            if !users.contains_key(name) && users.len() >= MAX_USERS {
                return Err(AclError::TooLarge);
            }
            let user = users.entry(name.to_vec()).or_default();
            for token in tokens {
                if token.len() > MAX_RULE_BYTES || user.rules.len() >= MAX_RULES {
                    return Err(AclError::TooLarge);
                }
                apply_rule(user, token.as_bytes()).map_err(|error| match error {
                    AclError::Syntax => AclError::Syntax,
                    other => other,
                })?;
            }
        }
        if !users
            .get(b"default".as_slice())
            .is_some_and(|user| user.enabled)
        {
            return Err(AclError::MissingDefault);
        }
        self.users = users;
        Ok(())
    }

    /// Atomically replace the ACL file: temp file in the same directory, data
    /// fsync, rename, then directory fsync where supported.
    pub fn save_file(&self, path: &Path) -> Result<(), AclError> {
        let mut body = String::new();
        for (name, user) in &self.users {
            body.push_str(&user.acl_line(&String::from_utf8_lossy(name)));
            body.push('\n');
        }
        let directory = path.parent().unwrap_or_else(|| Path::new("."));
        let temporary = directory.join(format!(
            ".{}.tmp-{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("aclfile"),
            std::process::id()
        ));
        let result = (|| -> io::Result<()> {
            #[cfg(unix)]
            let mut file = {
                use std::os::unix::fs::OpenOptionsExt as _;
                fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&temporary)?
            };
            #[cfg(not(unix))]
            let mut file = fs::File::create(&temporary)?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)?;
            if let Ok(directory) = fs::File::open(directory) {
                let _ = directory.sync_all();
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|error| AclError::Io(error.to_string()))
    }

    pub fn aclfile(&self) -> Option<&Path> {
        self.aclfile.as_deref()
    }

    pub fn restricted(&self) -> bool {
        !self.open()
    }
}

fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .iter()
            .all(|byte| (b'!'..=b'~').contains(byte) && *byte != b'|')
}

fn parse_hash(bytes: &[u8]) -> Option<[u8; 32]> {
    if bytes.len() != 64 {
        return None;
    }
    let mut digest = [0u8; 32];
    for (index, chunk) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(chunk).ok()?;
        digest[index] = u8::from_str_radix(text, 16).ok()?;
    }
    Some(digest)
}

fn apply_rule(user: &mut User, rule: &[u8]) -> Result<(), AclError> {
    if rule.eq_ignore_ascii_case(b"on") {
        user.enabled = true;
    } else if rule.eq_ignore_ascii_case(b"off") {
        user.enabled = false;
    } else if rule.eq_ignore_ascii_case(b"nopass") {
        user.nopass = true;
        user.passwords.clear();
    } else if rule.eq_ignore_ascii_case(b"resetpass") {
        user.nopass = false;
        user.passwords.clear();
    } else if rule.eq_ignore_ascii_case(b"allcommands") || rule.eq_ignore_ascii_case(b"+@all") {
        user.rules.push(Rule::All(true));
    } else if rule.eq_ignore_ascii_case(b"nocommands") || rule.eq_ignore_ascii_case(b"-@all") {
        user.rules.push(Rule::All(false));
    } else if rule.eq_ignore_ascii_case(b"allkeys") {
        user.keys = vec![b"*".to_vec()];
    } else if rule.eq_ignore_ascii_case(b"resetkeys") {
        user.keys.clear();
    } else if rule.eq_ignore_ascii_case(b"allchannels") {
        user.channels = vec![b"*".to_vec()];
    } else if rule.eq_ignore_ascii_case(b"resetchannels") {
        user.channels.clear();
    } else if rule.eq_ignore_ascii_case(b"reset") {
        *user = User::default();
    } else if let Some(password) = rule.strip_prefix(b">") {
        if password.is_empty() || password.len() > MAX_PASSWORD_BYTES {
            return Err(AclError::InvalidPassword);
        }
        if user.passwords.len() >= MAX_RULES {
            return Err(AclError::TooLarge);
        }
        user.nopass = false;
        let digest = sha256::digest(password);
        if !user.passwords.contains(&digest) {
            user.passwords.push(digest);
        }
    } else if let Some(password) = rule.strip_prefix(b"<") {
        if password.is_empty() {
            return Err(AclError::InvalidPassword);
        }
        let digest = sha256::digest(password);
        user.passwords.retain(|stored| *stored != digest);
    } else if let Some(hash) = rule.strip_prefix(b"#") {
        let digest = parse_hash(hash).ok_or(AclError::InvalidPassword)?;
        user.nopass = false;
        if !user.passwords.contains(&digest) {
            user.passwords.push(digest);
        }
    } else if let Some(hash) = rule.strip_prefix(b"!") {
        let digest = parse_hash(hash).ok_or(AclError::InvalidPassword)?;
        user.passwords.retain(|stored| *stored != digest);
    } else if let Some(category) = rule.strip_prefix(b"+@") {
        if !discovery::CATEGORIES
            .iter()
            .any(|known| category.eq_ignore_ascii_case(known.as_bytes()))
        {
            return Err(AclError::UnknownCategory);
        }
        user.rules
            .push(Rule::Category(category.to_ascii_lowercase(), true));
    } else if let Some(category) = rule.strip_prefix(b"-@") {
        if !discovery::CATEGORIES
            .iter()
            .any(|known| category.eq_ignore_ascii_case(known.as_bytes()))
        {
            return Err(AclError::UnknownCategory);
        }
        user.rules
            .push(Rule::Category(category.to_ascii_lowercase(), false));
    } else if let Some(command) = rule.strip_prefix(b"+") {
        let (name, subcommand) = split_command(command)?;
        user.rules.push(Rule::Command(name, subcommand, true));
    } else if let Some(command) = rule.strip_prefix(b"-") {
        let (name, subcommand) = split_command(command)?;
        user.rules.push(Rule::Command(name, subcommand, false));
    } else if let Some(pattern) = rule.strip_prefix(b"~") {
        if pattern.len() > MAX_PATTERN_BYTES || user.keys.len() >= MAX_PATTERNS {
            return Err(AclError::TooLarge);
        }
        if !user.keys.iter().any(|existing| existing == pattern) {
            user.keys.push(pattern.to_vec());
        }
    } else if let Some(pattern) = rule.strip_prefix(b"&") {
        if pattern.len() > MAX_PATTERN_BYTES || user.channels.len() >= MAX_PATTERNS {
            return Err(AclError::TooLarge);
        }
        if !user.channels.iter().any(|existing| existing == pattern) {
            user.channels.push(pattern.to_vec());
        }
    } else {
        return Err(AclError::UnknownRule);
    }
    Ok(())
}

fn split_command(command: &[u8]) -> Result<(Vec<u8>, Option<Vec<u8>>), AclError> {
    if command.is_empty() {
        return Err(AclError::UnknownCommand);
    }
    let (name, subcommand) = match command.iter().position(|byte| *byte == b'|') {
        Some(index) if index + 1 < command.len() => {
            (&command[..index], Some(&command[index + 1..]))
        }
        Some(_) => return Err(AclError::UnknownCommand),
        None => (command, None),
    };
    let Some(name) = discovery::canonical_name(name) else {
        return Err(AclError::UnknownCommand);
    };
    Ok((name.as_bytes().to_vec(), subcommand.map(<[u8]>::to_vec)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_user_password_and_nopass_semantics_are_explicit() {
        let mut auth = Auth::from_password(Some(b"pw"));
        assert!(auth.required());
        assert!(!auth.open());
        assert_eq!(auth.verify(None, b"pw"), Ok(()));
        assert_eq!(auth.verify(Some(b"default"), b"pw"), Ok(()));
        assert_eq!(
            auth.verify(Some(b"other"), b"pw"),
            Err(AuthError::WrongPassword)
        );
        auth.set_password(None);
        assert!(!auth.required());
        assert!(auth.open());
        assert_eq!(auth.verify(None, b""), Err(AuthError::NoPassword));
    }

    #[test]
    fn ordered_rules_allow_categories_commands_keys_and_explicit_overrides() {
        let mut auth = Auth::from_password(None);
        let rules: &[&[u8]] = &[
            b"on",
            b"nopass",
            b"resetkeys",
            b"~app:*",
            b"resetchannels",
            b"&events:*",
            b"-@all",
            b"+@read",
            b"+set",
        ];
        auth.setuser(b"reader", rules).unwrap();
        let user = auth.lookup(b"reader").unwrap();
        assert!(user.allows_command(&[b"GET", b"app:key"]));
        assert!(user.allows_command(&[b"SET", b"app:key"]));
        assert!(!user.allows_command(&[b"DEL", b"app:key"]));
        assert!(user.allows_key(b"app:key"));
        assert!(!user.allows_key(b"other:key"));
        assert_eq!(
            auth.authorize(Some(b"reader"), &[b"GET", b"other:key"]),
            Access::DeniedKey
        );
        assert_eq!(
            auth.authorize(Some(b"reader"), &[b"SET", b"app:key"]),
            Access::Allowed
        );
        assert_eq!(
            auth.authorize(Some(b"reader"), &[b"SET", b"other:key"]),
            Access::DeniedKey
        );
        assert_eq!(
            auth.authorize(Some(b"reader"), &[b"DEL", b"app:key"]),
            Access::DeniedCommand
        );
        auth.setuser(
            b"blocked",
            &[b"on", b"nopass", b"allkeys", b"-@all", b"+@read", b"-get"],
        )
        .unwrap();
        assert_eq!(
            auth.authorize(Some(b"blocked"), &[b"GET", b"k"]),
            Access::DeniedCommand
        );
        assert_eq!(
            auth.authorize(Some(b"blocked"), &[b"EXISTS", b"k"]),
            Access::Allowed
        );
    }

    #[test]
    fn disabled_users_and_default_password_gate_sessions() {
        let mut auth = Auth::from_password(None);
        auth.setuser(
            b"alice",
            &[b"on", b">secret", b"allkeys", b"allchannels", b"+@all"],
        )
        .unwrap();
        assert_eq!(auth.verify(Some(b"alice"), b"secret"), Ok(()));
        assert_eq!(
            auth.verify(Some(b"alice"), b"wrong"),
            Err(AuthError::WrongPassword)
        );
        assert_eq!(
            auth.authorize(Some(b"alice"), &[b"GET", b"k"]),
            Access::Allowed
        );
        auth.setuser(b"alice", &[b"off"]).unwrap();
        assert_eq!(
            auth.authorize(Some(b"alice"), &[b"GET", b"k"]),
            Access::NoAuth
        );
        auth.set_password(Some(b"pw"));
        assert_eq!(auth.authorize(None, &[b"GET", b"k"]), Access::NoAuth);
        assert_eq!(
            auth.authorize(Some(b"default"), &[b"GET", b"k"]),
            Access::Allowed
        );
    }

    #[test]
    fn setuser_is_atomic_on_invalid_rules_and_protects_names() {
        let mut auth = Auth::from_password(None);
        assert_eq!(
            auth.setuser(b"bob", &[b"on", b"+@bogus"]),
            Err(AclError::UnknownCategory)
        );
        assert!(auth.lookup(b"bob").is_none());
        assert_eq!(
            auth.setuser(b"bob", &[b"on", b"+no-such-command"]),
            Err(AclError::UnknownCommand)
        );
        assert!(auth.lookup(b"bob").is_none());
        assert_eq!(
            auth.setuser(b"has space", &[b"on"]),
            Err(AclError::InvalidName)
        );
    }

    #[test]
    fn acl_file_round_trips_hashes_rules_keys_and_channels() {
        let mut auth = Auth::from_password(None);
        auth.setuser(
            b"alice",
            &[
                b"on",
                b">password",
                b"resetkeys",
                b"~app:*",
                b"resetchannels",
                b"&events:*",
                b"-@all",
                b"+@read",
                b"+set|nx",
            ],
        )
        .unwrap();
        let path = std::env::temp_dir().join(format!(
            "rillcache-acl-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        auth.save_file(&path).unwrap();
        let loaded = Auth::from_acl_file(&path).unwrap();
        let alice = loaded.lookup(b"alice").unwrap();
        assert!(alice.enabled);
        assert!(!alice.nopass);
        assert_eq!(alice.keys, [b"app:*".to_vec()]);
        assert_eq!(alice.channels, [b"events:*".to_vec()]);
        assert!(loaded.verify(Some(b"alice"), b"password").is_ok());
        assert!(!alice.allows_command(&[b"DEL", b"app:k"]));
        assert!(alice.allows_command(&[b"GET", b"app:k"]));
        assert!(
            loaded
                .details(b"alice")
                .unwrap()
                .commands
                .contains("+@read")
        );
        std::fs::remove_file(&path).unwrap();
    }
}
