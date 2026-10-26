//! Bounded payload-budget eviction policies.
//!
//! The budget remains key and value bytes, not RSS. Eviction is attempted only
//! after atomic admission would otherwise fail, never for a noeviction policy, and
//! uses bounded candidate scans. LRU is approximated by mutation time because reads
//! do not take the store write lock.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EvictionPolicy {
    #[default]
    NoEviction,
    AllKeysRandom,
    AllKeysLru,
    VolatileRandom,
    VolatileLru,
    VolatileTtl,
}

impl EvictionPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "noeviction" => Some(Self::NoEviction),
            "allkeys-random" => Some(Self::AllKeysRandom),
            "allkeys-lru" => Some(Self::AllKeysLru),
            "volatile-random" => Some(Self::VolatileRandom),
            "volatile-lru" => Some(Self::VolatileLru),
            "volatile-ttl" => Some(Self::VolatileTtl),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::NoEviction => "noeviction",
            Self::AllKeysRandom => "allkeys-random",
            Self::AllKeysLru => "allkeys-lru",
            Self::VolatileRandom => "volatile-random",
            Self::VolatileLru => "volatile-lru",
            Self::VolatileTtl => "volatile-ttl",
        }
    }

    pub fn volatile(self) -> bool {
        matches!(
            self,
            Self::VolatileRandom | Self::VolatileLru | Self::VolatileTtl
        )
    }
}
