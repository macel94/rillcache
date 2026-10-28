//! Multi-user ACL rules with password digests and command/key permissions.
//!
//! The default user keeps the single-password behavior from S27. Additional users
//! add Redis-shaped `on/off`, password, command-category/command, read/write key,
//! channel and Redis 7 selector rules. This is not a complete Redis ACL
//! implementation: first-argument command rules, subcommand-specific categories,
//! persistent audit and rate limiting are absent and documented in
//! `docs/security.md`.

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

/// One key pattern with the access it grants. `~pattern`/`%RW~pattern` grant
/// both, `%R~pattern` only reads and `%W~pattern` only writes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct KeyPattern {
    pattern: Vec<u8>,
    access: discovery::KeyAccess,
}

impl KeyPattern {
    fn all() -> Self {
        Self {
            pattern: b"*".to_vec(),
            access: discovery::KeyAccess::ReadWrite,
        }
    }
}

/// One Redis 7 selector: an independent command/key/channel rule set.
#[derive(Clone, Debug, Default)]
struct Selector {
    rules: Vec<Rule>,
    keys: Vec<KeyPattern>,
    channels: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct User {
    enabled: bool,
    nopass: bool,
    passwords: Vec<[u8; 32]>,
    rules: Vec<Rule>,
    keys: Vec<KeyPattern>,
    channels: Vec<Vec<u8>>,
    selectors: Vec<Selector>,
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
            selectors: Vec::new(),
        }
    }
}

fn rules_allow(rules: &[Rule], arguments: &[&[u8]]) -> bool {
    let Some(command) = arguments.first() else {
        return false;
    };
    let subcommand = arguments.get(1);
    let mut allowed = false;
    for rule in rules {
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
                Rule::All(value) | Rule::Category(_, value) | Rule::Command(_, _, value) => *value,
            };
        }
    }
    allowed
}

fn patterns_allow(patterns: &[Vec<u8>], value: &[u8]) -> bool {
    patterns
        .iter()
        .any(|pattern| super::keyspace::glob_match(pattern, value))
}

fn keys_allow(keys: &[KeyPattern], key: &[u8], required: discovery::KeyAccess) -> bool {
    keys.iter().any(|pattern| {
        pattern.access.grants(required) && super::keyspace::glob_match(&pattern.pattern, key)
    })
}

fn key_access_allow(keys: &[KeyPattern], key: &[u8], access: discovery::KeyAccess) -> bool {
    match access {
        discovery::KeyAccess::Read => keys_allow(keys, key, discovery::KeyAccess::Read),
        discovery::KeyAccess::Write => keys_allow(keys, key, discovery::KeyAccess::Write),
        discovery::KeyAccess::ReadWrite => {
            keys_allow(keys, key, discovery::KeyAccess::Read)
                && keys_allow(keys, key, discovery::KeyAccess::Write)
        }
    }
}

fn add_key_pattern(
    keys: &mut Vec<KeyPattern>,
    pattern: &[u8],
    access: discovery::KeyAccess,
) -> Result<(), AclError> {
    if pattern.is_empty() {
        return Err(AclError::UnknownRule);
    }
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err(AclError::TooLarge);
    }
    if let Some(existing) = keys.iter_mut().find(|existing| existing.pattern == pattern) {
        if existing.access != access {
            existing.access = discovery::KeyAccess::ReadWrite;
        }
        return Ok(());
    }
    if keys.len() >= MAX_PATTERNS {
        return Err(AclError::TooLarge);
    }
    keys.push(KeyPattern {
        pattern: pattern.to_vec(),
        access,
    });
    Ok(())
}

fn key_patterns_string(keys: &[KeyPattern]) -> String {
    keys.iter()
        .map(|entry| {
            let prefix = match entry.access {
                discovery::KeyAccess::Read => "%R~",
                discovery::KeyAccess::Write => "%W~",
                discovery::KeyAccess::ReadWrite => "~",
            };
            format!("{prefix}{}", String::from_utf8_lossy(&entry.pattern))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn channel_patterns_string(channels: &[Vec<u8>]) -> String {
    channels
        .iter()
        .map(|pattern| format!("&{}", String::from_utf8_lossy(pattern)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn selector_line(selector: &Selector) -> String {
    let mut parts = Vec::new();
    if !selector.rules.is_empty() {
        parts.push(rules_text(&selector.rules));
    }
    if !selector.keys.is_empty() {
        parts.push(key_patterns_string(&selector.keys));
    }
    if !selector.channels.is_empty() {
        parts.push(channel_patterns_string(&selector.channels));
    }
    format!("({})", parts.join(" "))
}

fn rules_text(rules: &[Rule]) -> String {
    if rules.is_empty() {
        return "-@all".into();
    }
    rules
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

impl User {
    #[cfg(test)]
    fn allows_command(&self, arguments: &[&[u8]]) -> bool {
        rules_allow(&self.rules, arguments)
    }

    #[cfg(test)]
    fn allows_key(&self, key: &[u8], access: discovery::KeyAccess) -> bool {
        key_access_allow(&self.keys, key, access)
    }

    fn rules_string(&self) -> String {
        rules_text(&self.rules)
    }

    fn keys_string(&self) -> String {
        key_patterns_string(&self.keys)
    }

    fn channels_string(&self) -> String {
        channel_patterns_string(&self.channels)
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
        let mut line = format!(
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
        );
        for selector in &self.selectors {
            line.push(' ');
            line.push_str(&selector_line(selector));
        }
        line
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
    /// `(commands, keys, channels)` text for each selector.
    pub selectors: Vec<(String, String, String)>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Allowed,
    NoAuth,
    DeniedCommand,
    DeniedKey(Vec<u8>),
    DeniedChannel(Vec<u8>),
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

fn ruleset_access(
    rules: &[Rule],
    keys: &[KeyPattern],
    channels: &[Vec<u8>],
    arguments: &[&[u8]],
) -> Access {
    if !rules_allow(rules, arguments) {
        return Access::DeniedCommand;
    }
    let Some(command) = arguments.first() else {
        return Access::Allowed;
    };
    let all_keys = keys
        .iter()
        .any(|entry| entry.pattern == b"*" && entry.access.grants(discovery::KeyAccess::Read));
    if (command.eq_ignore_ascii_case(b"SORT") || command.eq_ignore_ascii_case(b"SORT_RO"))
        && sort_uses_dynamic_keys(arguments)
        && !all_keys
    {
        return Access::DeniedKey(b"BY/GET".to_vec());
    }
    for (key, access) in discovery::key_accesses(command, arguments) {
        if !key_access_allow(keys, key, access) {
            return Access::DeniedKey(key.to_vec());
        }
    }
    let channel_arguments: &[&[u8]] =
        if command.eq_ignore_ascii_case(b"PUBLISH") || command.eq_ignore_ascii_case(b"SPUBLISH") {
            &arguments[1..arguments.len().min(2)]
        } else if command.eq_ignore_ascii_case(b"SUBSCRIBE")
            || command.eq_ignore_ascii_case(b"UNSUBSCRIBE")
            || command.eq_ignore_ascii_case(b"PSUBSCRIBE")
            || command.eq_ignore_ascii_case(b"PUNSUBSCRIBE")
            || command.eq_ignore_ascii_case(b"SSUBSCRIBE")
            || command.eq_ignore_ascii_case(b"SUNSUBSCRIBE")
        {
            &arguments[1..]
        } else {
            &[]
        };
    for channel in channel_arguments {
        if !patterns_allow(channels, channel) {
            return Access::DeniedChannel(channel.to_vec());
        }
    }
    Access::Allowed
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
            keys: vec![KeyPattern::all()],
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
                    && user.keys == [KeyPattern::all()]
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
        Self::authorize_user_ref(user, arguments)
    }

    /// Evaluate a named user's rules without changing connection state. Returns
    /// `None` for a missing or disabled user (used by ACL DRYRUN).
    pub fn authorize_user(&self, name: &[u8], arguments: &[&[u8]]) -> Option<Access> {
        let user = self.lookup_name(name)?;
        let user = self.users.get(&user)?;
        user.enabled
            .then(|| Self::authorize_user_ref(user, arguments))
    }

    fn authorize_user_ref(user: &User, arguments: &[&[u8]]) -> Access {
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
        let root = ruleset_access(&user.rules, &user.keys, &user.channels, arguments);
        if root == Access::Allowed {
            return Access::Allowed;
        }
        let mut command_allowed = root != Access::DeniedCommand;
        let mut key_denial = match &root {
            Access::DeniedKey(key) => Some(key.clone()),
            _ => None,
        };
        let mut channel_denial = match &root {
            Access::DeniedChannel(channel) => Some(channel.clone()),
            _ => None,
        };
        for selector in &user.selectors {
            match ruleset_access(
                &selector.rules,
                &selector.keys,
                &selector.channels,
                arguments,
            ) {
                Access::Allowed => return Access::Allowed,
                Access::DeniedCommand | Access::NoAuth => {}
                Access::DeniedKey(key) => {
                    command_allowed = true;
                    key_denial.get_or_insert(key);
                }
                Access::DeniedChannel(channel) => {
                    command_allowed = true;
                    channel_denial.get_or_insert(channel);
                }
            }
        }
        if !command_allowed {
            Access::DeniedCommand
        } else if let Some(key) = key_denial {
            Access::DeniedKey(key)
        } else if let Some(channel) = channel_denial {
            Access::DeniedChannel(channel)
        } else {
            Access::DeniedCommand
        }
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
        if user.keys == [KeyPattern::all()] {
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
            selectors: user
                .selectors
                .iter()
                .map(|selector| {
                    (
                        if selector.rules.is_empty() {
                            String::new()
                        } else {
                            rules_text(&selector.rules)
                        },
                        key_patterns_string(&selector.keys),
                        channel_patterns_string(&selector.channels),
                    )
                })
                .collect(),
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
            let tokens = acl_tokens(line).ok_or(AclError::Syntax)?;
            let mut tokens = tokens.iter();
            if tokens.next().map(String::as_str) != Some("user") {
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
                apply_rule(user, token.as_bytes())?;
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

/// Split an ACL-file line into tokens, keeping a parenthesized selector
/// (which may contain spaces) as one token.
fn acl_tokens(line: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0u32;
    for ch in line.chars() {
        match ch {
            '(' => {
                depth = depth.checked_add(1)?;
                current.push(ch);
            }
            ')' => {
                depth = depth.checked_sub(1)?;
                current.push(ch);
            }
            ch if ch.is_ascii_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ch => current.push(ch),
        }
    }
    if depth != 0 {
        return None;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Some(tokens)
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
        user.keys = vec![KeyPattern::all()];
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
    } else if let Some(pattern) = rule.strip_prefix(b"%R~") {
        add_key_pattern(&mut user.keys, pattern, discovery::KeyAccess::Read)?;
    } else if let Some(pattern) = rule.strip_prefix(b"%W~") {
        add_key_pattern(&mut user.keys, pattern, discovery::KeyAccess::Write)?;
    } else if let Some(pattern) = rule.strip_prefix(b"%RW~") {
        add_key_pattern(&mut user.keys, pattern, discovery::KeyAccess::ReadWrite)?;
    } else if let Some(pattern) = rule.strip_prefix(b"~") {
        add_key_pattern(&mut user.keys, pattern, discovery::KeyAccess::ReadWrite)?;
    } else if let Some(pattern) = rule.strip_prefix(b"&") {
        if pattern.len() > MAX_PATTERN_BYTES || user.channels.len() >= MAX_PATTERNS {
            return Err(AclError::TooLarge);
        }
        if !user.channels.iter().any(|existing| existing == pattern) {
            user.channels.push(pattern.to_vec());
        }
    } else if rule.eq_ignore_ascii_case(b"clearselectors") {
        user.selectors.clear();
    } else if rule.first() == Some(&b'(') && rule.last() == Some(&b')') {
        if user.selectors.len() >= MAX_RULES {
            return Err(AclError::TooLarge);
        }
        let mut selector = Selector::default();
        for token in rule[1..rule.len() - 1]
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|token| !token.is_empty())
        {
            if token.len() > MAX_RULE_BYTES {
                return Err(AclError::TooLarge);
            }
            apply_selector_rule(&mut selector, token)?;
        }
        user.selectors.push(selector);
    } else {
        return Err(AclError::UnknownRule);
    }
    Ok(())
}

fn apply_selector_rule(selector: &mut Selector, rule: &[u8]) -> Result<(), AclError> {
    if rule.eq_ignore_ascii_case(b"allkeys") {
        selector.keys = vec![KeyPattern::all()];
    } else if rule.eq_ignore_ascii_case(b"resetkeys") {
        selector.keys.clear();
    } else if rule.eq_ignore_ascii_case(b"allchannels") {
        selector.channels = vec![b"*".to_vec()];
    } else if rule.eq_ignore_ascii_case(b"resetchannels") {
        selector.channels.clear();
    } else if let Some(category) = rule.strip_prefix(b"+@") {
        if !discovery::CATEGORIES
            .iter()
            .any(|known| category.eq_ignore_ascii_case(known.as_bytes()))
        {
            return Err(AclError::UnknownCategory);
        }
        if selector.rules.len() >= MAX_RULES {
            return Err(AclError::TooLarge);
        }
        selector
            .rules
            .push(Rule::Category(category.to_ascii_lowercase(), true));
    } else if let Some(category) = rule.strip_prefix(b"-@") {
        if !discovery::CATEGORIES
            .iter()
            .any(|known| category.eq_ignore_ascii_case(known.as_bytes()))
        {
            return Err(AclError::UnknownCategory);
        }
        if selector.rules.len() >= MAX_RULES {
            return Err(AclError::TooLarge);
        }
        selector
            .rules
            .push(Rule::Category(category.to_ascii_lowercase(), false));
    } else if let Some(command) = rule.strip_prefix(b"+") {
        if selector.rules.len() >= MAX_RULES {
            return Err(AclError::TooLarge);
        }
        let (name, subcommand) = split_command(command)?;
        selector.rules.push(Rule::Command(name, subcommand, true));
    } else if let Some(command) = rule.strip_prefix(b"-") {
        if selector.rules.len() >= MAX_RULES {
            return Err(AclError::TooLarge);
        }
        let (name, subcommand) = split_command(command)?;
        selector.rules.push(Rule::Command(name, subcommand, false));
    } else if let Some(pattern) = rule.strip_prefix(b"%R~") {
        add_key_pattern(&mut selector.keys, pattern, discovery::KeyAccess::Read)?;
    } else if let Some(pattern) = rule.strip_prefix(b"%W~") {
        add_key_pattern(&mut selector.keys, pattern, discovery::KeyAccess::Write)?;
    } else if let Some(pattern) = rule.strip_prefix(b"%RW~") {
        add_key_pattern(&mut selector.keys, pattern, discovery::KeyAccess::ReadWrite)?;
    } else if let Some(pattern) = rule.strip_prefix(b"~") {
        add_key_pattern(&mut selector.keys, pattern, discovery::KeyAccess::ReadWrite)?;
    } else if let Some(pattern) = rule.strip_prefix(b"&") {
        if pattern.len() > MAX_PATTERN_BYTES || selector.channels.len() >= MAX_PATTERNS {
            return Err(AclError::TooLarge);
        }
        if !selector.channels.iter().any(|existing| existing == pattern) {
            selector.channels.push(pattern.to_vec());
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

fn sort_uses_dynamic_keys(arguments: &[&[u8]]) -> bool {
    let mut index = 2;
    while index < arguments.len() {
        if arguments[index].eq_ignore_ascii_case(b"BY") {
            return arguments
                .get(index + 1)
                .is_some_and(|pattern| pattern.contains(&b'*'));
        }
        if arguments[index].eq_ignore_ascii_case(b"GET") {
            return true;
        }
        index += 1;
    }
    false
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
        assert!(user.allows_key(b"app:key", discovery::KeyAccess::ReadWrite));
        assert!(user.allows_key(b"app:key", discovery::KeyAccess::Read));
        assert!(!user.allows_key(b"other:key", discovery::KeyAccess::Read));
        assert_eq!(
            auth.authorize(Some(b"reader"), &[b"GET", b"other:key"]),
            Access::DeniedKey(b"other:key".to_vec())
        );
        assert_eq!(
            auth.authorize(Some(b"reader"), &[b"SET", b"app:key"]),
            Access::Allowed
        );
        assert_eq!(
            auth.authorize(Some(b"reader"), &[b"SET", b"other:key"]),
            Access::DeniedKey(b"other:key".to_vec())
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
    fn first_argument_rules_allow_and_deny_subcommands_case_insensitively() {
        let mut auth = Auth::from_password(None);
        auth.setuser(
            b"config-reader",
            &[
                b"on",
                b"nopass",
                b"allkeys",
                b"resetchannels",
                b"-@all",
                b"+CONFIG|GeT",
            ],
        )
        .unwrap();
        let user = auth.lookup(b"config-reader").unwrap();
        assert!(user.allows_command(&[b"CONFIG", b"GET", b"maxmemory"]));
        assert!(!user.allows_command(&[b"CONFIG", b"SET", b"maxmemory", b"1"]));
        assert!(!user.allows_command(&[b"CONFIG"]));
        assert!(!user.allows_command(&[b"GET", b"any"]));
        assert!(rules_text(&user.rules).contains("+config|GeT"));

        auth.setuser(
            b"config-except-set",
            &[
                b"on",
                b"nopass",
                b"allkeys",
                b"resetchannels",
                b"-@all",
                b"+config",
                b"-config|set",
            ],
        )
        .unwrap();
        let user = auth.lookup(b"config-except-set").unwrap();
        assert!(user.allows_command(&[b"CONFIG", b"GET", b"maxmemory"]));
        assert!(!user.allows_command(&[b"CONFIG", b"SET", b"maxmemory", b"1"]));
        assert!(user.allows_command(&[b"CONFIG", b"RESETSTAT"]));

        auth.setuser(
            b"selector-reader",
            &[
                b"on",
                b"nopass",
                b"allkeys",
                b"resetchannels",
                b"-@all",
                b"(+config|get)",
            ],
        )
        .unwrap();
        assert_eq!(
            auth.authorize(Some(b"selector-reader"), &[b"CONFIG", b"GET", b"maxmemory"]),
            Access::Allowed
        );
        assert_eq!(
            auth.authorize(
                Some(b"selector-reader"),
                &[b"CONFIG", b"SET", b"maxmemory", b"1"]
            ),
            Access::DeniedCommand
        );

        assert!(
            auth.setuser(b"empty-sub", &[b"on", b"nopass", b"+config|"])
                .is_err()
        );
        assert!(
            auth.setuser(b"unknown-command", &[b"on", b"nopass", b"+bogus|get"])
                .is_err()
        );
    }

    #[test]
    fn sort_dynamic_patterns_require_all_key_access() {
        let mut auth = Auth::from_password(None);
        auth.setuser(
            b"restricted",
            &[
                b"on",
                b"nopass",
                b"~src",
                b"resetchannels",
                b"-@all",
                b"+sort",
                b"+sort_ro",
            ],
        )
        .unwrap();
        assert_eq!(
            auth.authorize(Some(b"restricted"), &[b"SORT", b"src", b"BY", b"weight_*"]),
            Access::DeniedKey(b"BY/GET".to_vec())
        );
        assert_eq!(
            auth.authorize(Some(b"restricted"), &[b"SORT_RO", b"src", b"GET", b"#"]),
            Access::DeniedKey(b"BY/GET".to_vec())
        );
        assert_eq!(
            auth.authorize(Some(b"restricted"), &[b"SORT", b"src", b"BY", b"nosort"]),
            Access::Allowed
        );
        auth.setuser(
            b"allkeys",
            &[
                b"on",
                b"nopass",
                b"allkeys",
                b"resetchannels",
                b"-@all",
                b"+sort",
            ],
        )
        .unwrap();
        assert_eq!(
            auth.authorize(Some(b"allkeys"), &[b"SORT", b"src", b"GET", b"#"]),
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
    fn key_modes_parse_merge_list_and_reject_unknown_prefixes() {
        let mut auth = Auth::from_password(None);
        auth.setuser(
            b"m",
            &[
                b"on",
                b"nopass",
                b"-@all",
                b"+get",
                b"+set",
                b"%R~read:*",
                b"%W~write:*",
                b"%RW~both:*",
                b"~legacy:*",
            ],
        )
        .unwrap();
        {
            let user = auth.users.get(b"m".as_slice()).unwrap();
            assert!(user.allows_key(b"read:1", discovery::KeyAccess::Read));
            assert!(!user.allows_key(b"read:1", discovery::KeyAccess::Write));
            assert!(user.allows_key(b"write:1", discovery::KeyAccess::Write));
            assert!(!user.allows_key(b"write:1", discovery::KeyAccess::Read));
            assert!(user.allows_key(b"both:1", discovery::KeyAccess::Read));
            assert!(user.allows_key(b"both:1", discovery::KeyAccess::Write));
            assert!(user.allows_key(b"legacy:1", discovery::KeyAccess::ReadWrite));
            assert!(!user.allows_key(b"other:1", discovery::KeyAccess::Read));
        }
        let line = auth.acl_line(b"m").unwrap();
        assert!(line.contains("%R~read:*"), "{line}");
        assert!(line.contains("%W~write:*"), "{line}");
        assert!(line.contains("~both:*"), "{line}");
        assert!(!line.contains("%RW~both:*"), "{line}");

        // Read and write grants for the same pattern merge into read-write.
        auth.setuser(b"m", &[b"%R~mix:*", b"%W~mix:*"]).unwrap();
        let line = auth.acl_line(b"m").unwrap();
        assert!(line.contains("~mix:*"), "{line}");
        assert!(!line.contains("%R~mix:*"), "{line}");

        // Unknown or malformed key-mode prefixes reject the whole update.
        assert_eq!(
            auth.setuser(b"m", &[b"%X~bad:*"]),
            Err(AclError::UnknownRule)
        );
        assert_eq!(auth.setuser(b"m", &[b"%R~"]), Err(AclError::UnknownRule));
        let line = auth.acl_line(b"m").unwrap();
        assert!(!line.contains("bad:*"), "{line}");
    }

    #[test]
    fn selectors_parse_clear_and_remain_atomic() {
        let mut auth = Auth::from_password(None);
        auth.setuser(b"s", &[b"on", b"nopass", b"(+get ~a*)", b"(+set ~b*)"])
            .unwrap();
        {
            let user = auth.users.get(b"s".as_slice()).unwrap();
            assert_eq!(user.selectors.len(), 2);
            assert_eq!(user.selectors[0].rules.len(), 1);
            assert_eq!(user.selectors[1].keys.len(), 1);
            assert_eq!(user.selectors[1].keys[0].pattern, b"b*");
            assert_eq!(
                user.selectors[1].keys[0].access,
                discovery::KeyAccess::ReadWrite
            );
        }
        assert_eq!(
            auth.setuser(b"s", &[b"(+get on)"]),
            Err(AclError::UnknownRule)
        );
        assert_eq!(auth.users.get(b"s".as_slice()).unwrap().selectors.len(), 2);
        auth.setuser(b"s", &[b"clearselectors"]).unwrap();
        assert!(
            auth.users
                .get(b"s".as_slice())
                .unwrap()
                .selectors
                .is_empty()
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
        assert_eq!(alice.keys.len(), 1);
        assert_eq!(alice.keys[0].pattern, b"app:*");
        assert_eq!(alice.keys[0].access, discovery::KeyAccess::ReadWrite);
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
