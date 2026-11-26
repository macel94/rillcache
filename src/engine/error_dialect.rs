//! Explicit selection for verified Redis/Valkey protocol and error divergences.
//! This is not a general server-identity or command-semantics compatibility mode.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ErrorDialect {
    Redis,
    /// Use the pinned Valkey wording for the selected command errors by default.
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

    pub(super) fn hgetdel_missing_fields(self) -> &'static [u8] {
        match self {
            Self::Redis => {
                b"-ERR Mandatory argument FIELDS is missing or not at the right position\r\n"
            }
            Self::Valkey => super::options::SYNTAX,
        }
    }

    pub(super) fn hgetdel_invalid_count(self) -> &'static [u8] {
        match self {
            Self::Redis => b"-ERR Number of fields must be a positive integer\r\n",
            Self::Valkey => super::options::NOT_INTEGER,
        }
    }

    pub(super) fn hgetdel_wrong_count(self, nonpositive: bool) -> &'static [u8] {
        match self {
            Self::Redis if nonpositive => self.hgetdel_invalid_count(),
            Self::Redis => b"-ERR The `numfields` parameter must match the number of arguments\r\n",
            Self::Valkey => b"-ERR numfields should be greater than 0 and match the provided number of fields\r\n",
        }
    }

    pub(super) fn geo_shape_required(self, store: bool) -> &'static [u8] {
        match (self, store) {
            (Self::Redis, false) => {
                b"-ERR exactly one of BYRADIUS and BYBOX can be specified for GEOSEARCH\r\n"
            }
            (Self::Redis, true) => {
                b"-ERR exactly one of BYRADIUS and BYBOX can be specified for GEOSEARCHSTORE\r\n"
            }
            (Self::Valkey, false) => {
                b"-ERR exactly one of BYRADIUS, BYBOX and BYPOLYGON can be specified for GEOSEARCH\r\n"
            }
            (Self::Valkey, true) => {
                b"-ERR exactly one of BYRADIUS, BYBOX and BYPOLYGON can be specified for GEOSEARCHSTORE\r\n"
            }
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

    #[test]
    fn geo_search_missing_shape_text_tracks_the_selected_vendor() {
        assert_eq!(
            ErrorDialect::Redis.geo_shape_required(false),
            b"-ERR exactly one of BYRADIUS and BYBOX can be specified for GEOSEARCH\r\n"
        );
        assert_eq!(
            ErrorDialect::Valkey.geo_shape_required(false),
            b"-ERR exactly one of BYRADIUS, BYBOX and BYPOLYGON can be specified for GEOSEARCH\r\n"
        );
    }
}
