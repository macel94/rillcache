//! Explicit selection for verified Redis/Valkey error-text divergences.
//! This is not a general server-identity or command-semantics compatibility mode.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ErrorDialect {
    Redis,
    /// Preserve Rillcache's existing MSETEX short-pair error by default.
    #[default]
    Valkey,
}

impl ErrorDialect {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "redis" => Some(Self::Redis),
            "valkey" => Some(Self::Valkey),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Redis => "redis",
            Self::Valkey => "valkey",
        }
    }

    pub(super) fn short_msetex_pairs(self) -> &'static [u8] {
        match self {
            Self::Redis => b"-ERR wrong number of key-value pairs\r\n",
            Self::Valkey => super::options::SYNTAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialect_names_are_explicit_and_default_is_backward_compatible() {
        assert_eq!(ErrorDialect::default(), ErrorDialect::Valkey);
        for dialect in [ErrorDialect::Redis, ErrorDialect::Valkey] {
            assert_eq!(ErrorDialect::parse(dialect.as_str()), Some(dialect));
        }
        for value in ["", "auto", "REDIS", "redis\0", "valkey "] {
            assert_eq!(ErrorDialect::parse(value), None);
        }
    }
}
