//! Redis-shaped keyspace notification flags and event categories.
//!
//! Supported flags are `K`, `E`, `g`, `$`, `l`, `s`, `h`, `z`, `t` and the `A`
//! alias for all supported categories. Expired/evicted/keymiss/new-key/module
//! classes are deliberately rejected instead of silently claiming them.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotifyFlags {
    pub keyspace: bool,
    pub keyevent: bool,
    pub generic: bool,
    pub string: bool,
    pub list: bool,
    pub set: bool,
    pub hash: bool,
    pub zset: bool,
    pub stream: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Generic,
    String,
    List,
    Set,
    Hash,
    Zset,
    Stream,
}

impl NotifyFlags {
    pub fn parse(raw: &[u8]) -> Option<Self> {
        let mut flags = Self::default();
        for byte in raw {
            match byte {
                b'K' => flags.keyspace = true,
                b'E' => flags.keyevent = true,
                b'g' => flags.generic = true,
                b'$' => flags.string = true,
                b'l' => flags.list = true,
                b's' => flags.set = true,
                b'h' => flags.hash = true,
                b'z' => flags.zset = true,
                b't' => flags.stream = true,
                b'A' => {
                    flags.generic = true;
                    flags.string = true;
                    flags.list = true;
                    flags.set = true;
                    flags.hash = true;
                    flags.zset = true;
                    flags.stream = true;
                }
                _ => return None,
            }
        }
        Some(flags)
    }

    pub fn as_string(self) -> String {
        let mut text = String::new();
        if self.keyspace {
            text.push('K');
        }
        if self.keyevent {
            text.push('E');
        }
        for (enabled, flag) in [
            (self.generic, 'g'),
            (self.string, '$'),
            (self.list, 'l'),
            (self.set, 's'),
            (self.hash, 'h'),
            (self.zset, 'z'),
            (self.stream, 't'),
        ] {
            if enabled {
                text.push(flag);
            }
        }
        text
    }

    pub fn bits(self) -> u32 {
        u32::from(self.keyspace)
            | (u32::from(self.keyevent) << 1)
            | (u32::from(self.generic) << 2)
            | (u32::from(self.string) << 3)
            | (u32::from(self.list) << 4)
            | (u32::from(self.set) << 5)
            | (u32::from(self.hash) << 6)
            | (u32::from(self.zset) << 7)
            | (u32::from(self.stream) << 8)
    }

    pub fn from_bits(bits: u32) -> Self {
        Self {
            keyspace: bits & 1 != 0,
            keyevent: bits & 2 != 0,
            generic: bits & 4 != 0,
            string: bits & 8 != 0,
            list: bits & 16 != 0,
            set: bits & 32 != 0,
            hash: bits & 64 != 0,
            zset: bits & 128 != 0,
            stream: bits & 256 != 0,
        }
    }

    pub fn is_empty(self) -> bool {
        !(self.keyspace
            || self.keyevent
            || self.generic
            || self.string
            || self.list
            || self.set
            || self.hash
            || self.zset
            || self.stream)
    }

    pub fn category(self, category: Category) -> bool {
        match category {
            Category::Generic => self.generic,
            Category::String => self.string,
            Category::List => self.list,
            Category::Set => self.set,
            Category::Hash => self.hash,
            Category::Zset => self.zset,
            Category::Stream => self.stream,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_round_trip_and_reject_unsupported_classes() {
        let flags = NotifyFlags::parse(b"KEA").unwrap();
        assert!(flags.keyspace && flags.keyevent);
        assert!(flags.category(Category::Generic));
        assert!(flags.category(Category::Stream));
        assert_eq!(flags.as_string(), "KEg$lshzt");
        assert_eq!(NotifyFlags::parse(b"KEg$lshzt").unwrap(), flags);
        assert!(NotifyFlags::parse(b"x").is_none());
        assert!(NotifyFlags::parse(b"Q").is_none());
        assert!(NotifyFlags::default().is_empty());
    }
}
