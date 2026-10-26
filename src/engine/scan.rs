//! Bounded cursor iteration for the keyspace and collection types.
//!
//! Cursors are decimal indexes into a freshly sorted snapshot per call. Like
//! Redis SCAN, the iteration is only a bounded traversal hint: concurrent
//! insertions/deletions can produce duplicates or misses. For a stable keyspace
//! every element is returned exactly once. No cursor state is retained between
//! calls.

use super::data::Collection;
use super::keyspace::glob_match;
use super::{Store, arity, encode_bulk, internal_error, wrong_type};
use std::io::Write;

const DEFAULT_COUNT: usize = 10;
const MAX_COUNT: usize = 10_000;

struct Options {
    cursor: usize,
    count: usize,
    pattern: Option<Vec<u8>>,
    kind: Option<Vec<u8>>,
    no_values: bool,
}

impl Options {
    fn parse(
        arguments: &[&[u8]],
        start: usize,
        scan_type: bool,
        scan_hash: bool,
    ) -> Result<Self, ()> {
        let raw = arguments.get(start).ok_or(())?;
        let cursor = parse_cursor(raw).ok_or(())?;
        let mut options = Self {
            cursor,
            count: DEFAULT_COUNT,
            pattern: None,
            kind: None,
            no_values: false,
        };
        let mut index = start + 1;
        while index < arguments.len() {
            let option = arguments[index];
            if option.eq_ignore_ascii_case(b"MATCH") {
                index += 1;
                options.pattern = Some(arguments.get(index).ok_or(())?.to_vec());
            } else if option.eq_ignore_ascii_case(b"COUNT") {
                index += 1;
                options.count = parse_count(arguments.get(index).ok_or(())?)?;
            } else if scan_type && option.eq_ignore_ascii_case(b"TYPE") {
                index += 1;
                options.kind = Some(arguments.get(index).ok_or(())?.to_vec());
            } else if scan_hash && option.eq_ignore_ascii_case(b"NOVALUES") {
                options.no_values = true;
            } else {
                return Err(());
            }
            index += 1;
        }
        Ok(options)
    }

    fn wants(&self, value: &[u8]) -> bool {
        self.pattern
            .as_ref()
            .is_none_or(|pattern| glob_match(pattern, value))
    }
}

pub(super) fn parse_cursor(raw: &[u8]) -> Option<usize> {
    if raw.is_empty() || !raw.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(raw).ok()?.parse().ok()
}

fn parse_count(raw: &[u8]) -> Result<usize, ()> {
    let count = parse_cursor(raw).filter(|count| *count > 0).ok_or(())?;
    Ok(count.min(MAX_COUNT))
}

impl Store {
    pub(super) fn execute_scan(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> bool {
        let Some(command) = arguments.first() else {
            return false;
        };
        if command.eq_ignore_ascii_case(b"SCAN") {
            if arguments.len() < 2 {
                arity("scan", output);
            } else {
                self.scan_keys(arguments, output);
            }
            return true;
        }
        let (name, kind) = if command.eq_ignore_ascii_case(b"HSCAN") {
            ("hscan", Kind::Hash)
        } else if command.eq_ignore_ascii_case(b"SSCAN") {
            ("sscan", Kind::Set)
        } else if command.eq_ignore_ascii_case(b"ZSCAN") {
            ("zscan", Kind::ZSet)
        } else {
            return false;
        };
        if arguments.len() < 3 {
            arity(name, output);
        } else {
            self.scan_collection(arguments, kind, output);
        }
        true
    }

    fn scan_keys(&self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let Ok(options) = Options::parse(arguments, 1, true, false) else {
            output.extend_from_slice(b"-ERR syntax error or invalid cursor\r\n");
            return;
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.reap(now, usize::MAX);
        let mut keys: Vec<&[u8]> = data.entries.keys().map(Vec::as_slice).collect();
        keys.sort_unstable();
        let start = options.cursor.min(keys.len());
        let end = start.saturating_add(options.count).min(keys.len());
        let mut selected = Vec::new();
        for key in &keys[start..end] {
            if !options.wants(key) {
                continue;
            }
            if let Some(kind) = &options.kind {
                let entry = data.entries.get(*key).expect("snapshot entry");
                if !type_matches(entry, kind) {
                    continue;
                }
            }
            selected.push(*key);
        }
        let next = if end >= keys.len() { 0 } else { end };
        write_cursor_array(next, &selected, output);
    }

    fn scan_collection(&self, arguments: &[&[u8]], kind: Kind, output: &mut Vec<u8>) {
        let name = kind.name();
        let Ok(options) = Options::parse(arguments, 2, false, kind == Kind::Hash) else {
            output.extend_from_slice(b"-ERR syntax error or invalid cursor\r\n");
            return;
        };
        let key = arguments[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            write_cursor_array(0, &[], output);
            return;
        };
        let collection = match entry.collection.as_ref() {
            Some(collection) if kind.matches(collection) => collection,
            _ => {
                wrong_type(output);
                return;
            }
        };
        match kind {
            Kind::Hash => {
                let mut fields = collection
                    .hash_items()
                    .expect("hash type checked")
                    .into_iter()
                    .map(|(field, value)| (field.to_vec(), value.to_vec()))
                    .collect::<Vec<_>>();
                fields.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                let start = options.cursor.min(fields.len());
                let end = start.saturating_add(options.count).min(fields.len());
                let mut selected = Vec::new();
                for (field, value) in &fields[start..end] {
                    if options.wants(field) {
                        selected.push((field.as_slice(), Some(value.as_slice())));
                    }
                }
                let next = if end >= fields.len() { 0 } else { end };
                if options.no_values {
                    let items: Vec<&[u8]> = selected.iter().map(|(field, _)| *field).collect();
                    write_cursor_array(next, &items, output);
                } else {
                    write_cursor_pairs(next, &selected, output);
                }
            }
            Kind::Set => {
                let mut members = collection.set_items().expect("set type checked");
                members.sort_unstable();
                let start = options.cursor.min(members.len());
                let end = start.saturating_add(options.count).min(members.len());
                let selected: Vec<&[u8]> = members[start..end]
                    .iter()
                    .filter(|member| options.wants(member))
                    .map(Vec::as_slice)
                    .collect();
                let next = if end >= members.len() { 0 } else { end };
                write_cursor_array(next, &selected, output);
            }
            Kind::ZSet => {
                let mut members = collection
                    .zset_items()
                    .expect("zset type checked")
                    .into_iter()
                    .map(|(member, score)| (member.to_vec(), score))
                    .collect::<Vec<_>>();
                members.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                let start = options.cursor.min(members.len());
                let end = start.saturating_add(options.count).min(members.len());
                let scores: Vec<String> = members[start..end]
                    .iter()
                    .map(|(_, score)| score.to_string())
                    .collect();
                let selected: Vec<(&[u8], Option<&[u8]>)> = members[start..end]
                    .iter()
                    .zip(&scores)
                    .filter(|((member, _), _)| options.wants(member))
                    .map(|((member, _), score)| (member.as_slice(), Some(score.as_bytes())))
                    .collect();
                let next = if end >= members.len() { 0 } else { end };
                write_cursor_pairs(next, &selected, output);
            }
        }
        let _ = name;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Hash,
    Set,
    ZSet,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Hash => "hscan",
            Self::Set => "sscan",
            Self::ZSet => "zscan",
        }
    }

    fn matches(self, collection: &Collection) -> bool {
        match self {
            Self::Hash => collection.is_hash(),
            Self::Set => collection.is_set(),
            Self::ZSet => collection.is_zset(),
        }
    }
}

fn type_matches(entry: &super::data::Entry, kind: &[u8]) -> bool {
    if kind.eq_ignore_ascii_case(b"string") {
        return entry.is_string();
    }
    if kind.eq_ignore_ascii_case(b"stream") {
        return entry.stream.is_some();
    }
    let Some(collection) = entry.collection.as_ref() else {
        return false;
    };
    if kind.eq_ignore_ascii_case(b"list") {
        collection.is_list()
    } else if kind.eq_ignore_ascii_case(b"set") {
        collection.is_set()
    } else if kind.eq_ignore_ascii_case(b"hash") {
        collection.is_hash()
    } else if kind.eq_ignore_ascii_case(b"zset") {
        collection.is_zset()
    } else {
        false
    }
}

fn write_cursor_array(next: usize, values: &[&[u8]], output: &mut Vec<u8>) {
    write!(
        output,
        "*2\r\n${}\r\n{next}\r\n*{}\r\n",
        next.to_string().len(),
        values.len()
    )
    .expect("Vec write");
    for value in values {
        encode_bulk(Some(value), output);
    }
}

fn write_cursor_pairs(next: usize, values: &[(&[u8], Option<&[u8]>)], output: &mut Vec<u8>) {
    let length = values.len()
        * if values.iter().all(|(_, value)| value.is_some()) {
            2
        } else {
            1
        };
    write!(
        output,
        "*2\r\n${}\r\n{next}\r\n*{length}\r\n",
        next.to_string().len()
    )
    .expect("Vec write");
    for (value, extra) in values {
        encode_bulk(Some(value), output);
        if let Some(extra) = extra {
            encode_bulk(Some(extra), output);
        }
    }
}
