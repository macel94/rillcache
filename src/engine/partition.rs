//! Immutable virtual-partition placement. Layouts are shared, Data is not.
//!
//! The router keeps its power-of-two partition contract while worker counts may
//! be any value from one through MAX_SHARDS. No live migration is represented.

use super::shard::{MAX_SHARDS, ShardRouter};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct OwnerId(usize);

impl OwnerId {
    pub(crate) fn new(index: usize) -> Self {
        Self(index)
    }

    pub(crate) fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayoutError {
    NoOwners,
    TooManyOwners(usize),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOwners => f.write_str("a storage layout needs at least one owner"),
            Self::TooManyOwners(n) => {
                write!(f, "storage owner count {n} exceeds {MAX_SHARDS}")
            }
        }
    }
}

impl std::error::Error for LayoutError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PartitionLayout {
    router: ShardRouter,
    owners: usize,
    placement: [OwnerId; MAX_SHARDS],
}

impl PartitionLayout {
    pub(crate) fn new(owners: usize) -> Result<Self, LayoutError> {
        Self::validate(owners)?;
        let partitions = if owners == 1 { 1 } else { MAX_SHARDS };
        Ok(Self::from_router(
            owners,
            ShardRouter::new(partitions).expect("fixed valid partition count"),
        ))
    }

    #[cfg(test)]
    fn with_seed(owners: usize, seed: u64) -> Result<Self, LayoutError> {
        Self::validate(owners)?;
        let partitions = if owners == 1 { 1 } else { MAX_SHARDS };
        Ok(Self::from_router(
            owners,
            ShardRouter::with_seed(partitions, seed).expect("fixed valid partition count"),
        ))
    }

    fn validate(owners: usize) -> Result<(), LayoutError> {
        if owners == 0 {
            return Err(LayoutError::NoOwners);
        }
        if owners > MAX_SHARDS {
            return Err(LayoutError::TooManyOwners(owners));
        }
        Ok(())
    }

    pub(crate) fn from_router(owners: usize, router: ShardRouter) -> Self {
        Self {
            router,
            owners,
            placement: std::array::from_fn(|partition| OwnerId(partition % owners)),
        }
    }

    pub(crate) fn owner_count(self) -> usize {
        self.owners
    }

    pub(crate) fn partition_count(self) -> usize {
        self.router.count()
    }

    pub(crate) fn partition_for(self, key: &[u8]) -> usize {
        self.router.shard_for(key)
    }

    pub(crate) fn owner_for(self, key: &[u8]) -> OwnerId {
        self.placement[self.partition_for(key)]
    }

    pub(crate) fn partition_owner(self, partition: usize) -> Option<OwnerId> {
        (partition < self.partition_count()).then(|| self.placement[partition])
    }

    pub(crate) fn owners(self) -> impl ExactSizeIterator<Item = OwnerId> {
        (0..self.owners).map(OwnerId)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_partitions_are_balanced_for_every_owner_count() {
        for n in 1..=MAX_SHARDS {
            let layout = PartitionLayout::new(n).unwrap();
            assert_eq!(layout.owner_count(), n);
            assert_eq!(layout.partition_count(), if n == 1 { 1 } else { 64 });
            let mut counts = vec![0; n];
            for partition in 0..layout.partition_count() {
                counts[layout.partition_owner(partition).unwrap().index()] += 1;
            }
            assert_eq!(layout.partition_owner(layout.partition_count()), None);
            assert!(counts.iter().all(|n| *n > 0));
            assert!(counts.iter().max().unwrap() - counts.iter().min().unwrap() <= 1);
            assert_eq!(
                layout.owners().map(OwnerId::index).collect::<Vec<_>>(),
                (0..n).collect::<Vec<_>>()
            );
        }
        assert_eq!(PartitionLayout::new(0), Err(LayoutError::NoOwners));
        assert_eq!(
            PartitionLayout::new(65),
            Err(LayoutError::TooManyOwners(65))
        );
    }

    #[test]
    fn binary_keys_have_one_stable_owner_and_seed_changes_are_visible() {
        let first = PartitionLayout::with_seed(3, 17).unwrap();
        let second = PartitionLayout::with_seed(3, 18).unwrap();
        let clone = first;
        for key in [b"".as_slice(), b"\0\xff\r\n", &vec![b'x'; 65536]] {
            assert_eq!(first.owner_for(key), clone.owner_for(key));
            assert!(first.owner_for(key).index() < 3);
            assert!(first.partition_for(key) < 64);
        }
        assert!(
            (0_u64..4096)
                .any(|n| first.owner_for(&n.to_le_bytes()) != second.owner_for(&n.to_le_bytes()))
        );
        let one = PartitionLayout::with_seed(1, 17).unwrap();
        for n in 0_u64..4096 {
            assert_eq!(one.owner_for(&n.to_le_bytes()).index(), 0);
            assert_eq!(one.partition_for(&n.to_le_bytes()), 0);
        }
    }
}
