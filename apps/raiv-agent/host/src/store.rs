//! Exact-byte bounded RAM storage. Placement/readback is neither durability nor acceptance.

use raiv::{
    format::{validate_drisl, CodecError, DaslCid, DecodeLimits},
    graph::{GraphError, MaterializedBlocks, RestoredAgent},
    limits::Limits,
};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreLimits {
    pub max_blocks: u64,
    pub max_bytes: u64,
    pub wire: DecodeLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    Codec(CodecError),
    Integrity,
    Capacity,
    Allocation,
    ConflictingBytes,
    Graph(GraphError),
    SnapshotMismatch,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Codec(e) => return e.fmt(f),
            Self::Integrity => "stored block bytes do not match structural CID",
            Self::Capacity => "RAM block store capacity reached",
            Self::Allocation => "RAM block store allocation failure",
            Self::ConflictingBytes => "existing CID has different bytes",
            Self::Graph(e) => return e.fmt(f),
            Self::SnapshotMismatch => "readback closure differs from supplied snapshot",
        })
    }
}

impl std::error::Error for StoreError {}

struct Block {
    cid: DaslCid,
    body: Vec<u8>,
}

/// Capacity is explicit native policy, independent of one Core closure's limits.
/// No hydration, eviction, alternate-byte normalization or implicit HEAD update.
pub struct MemoryBlockStore {
    blocks: Vec<Block>,
    bytes: u64,
    limits: StoreLimits,
}

impl MemoryBlockStore {
    pub fn new(limits: StoreLimits) -> Self {
        Self {
            blocks: Vec::new(),
            bytes: 0,
            limits,
        }
    }

    pub fn block_count(&self) -> u64 {
        self.blocks.len() as u64
    }

    pub fn encoded_bytes(&self) -> u64 {
        self.bytes
    }

    pub fn blocks(&self) -> impl Iterator<Item = (DaslCid, &[u8])> {
        self.blocks.iter().map(|b| (b.cid, b.body.as_slice()))
    }

    /// Validate before ownership. Equal CID/body is idempotent without another payload.
    pub fn put(&mut self, cid: DaslCid, body: &[u8]) -> Result<(), StoreError> {
        self.stage(core::iter::once((cid, body)))
    }

    /// Stage the entire unique required closure atomically; unchanged source blocks reuse
    /// their existing owned buffers. Failure keeps visible blocks/count/bytes unchanged.
    pub fn place(&mut self, snapshot: &RestoredAgent) -> Result<(), StoreError> {
        self.stage(snapshot.blocks())
    }

    /// Re-read the whole bounded required closure, including every existing dependency.
    /// This is a current RAM data check, not a retention lease or future availability proof.
    pub fn readback(&self, expected: &RestoredAgent, caller: &Limits) -> Result<(), StoreError> {
        let restored =
            RestoredAgent::restore(expected.root(), self, caller).map_err(StoreError::Graph)?;
        if restored != *expected {
            return Err(StoreError::SnapshotMismatch);
        }
        Ok(())
    }

    fn stage<'a>(
        &mut self,
        blocks: impl Iterator<Item = (DaslCid, &'a [u8])>,
    ) -> Result<(), StoreError> {
        let mut staged: Vec<Block> = Vec::new();
        let mut total = self.bytes;
        for (cid, body) in blocks {
            cid.require_state().map_err(|_| StoreError::Integrity)?;
            validate_drisl(body, &self.limits.wire).map_err(StoreError::Codec)?;
            cid.verify(body).map_err(|_| StoreError::Integrity)?;
            if let Some(existing) = self.blocks.iter().chain(&staged).find(|b| b.cid == cid) {
                if existing.body != body {
                    return Err(StoreError::ConflictingBytes);
                }
                continue;
            }
            let count = self
                .block_count()
                .checked_add(staged.len() as u64)
                .and_then(|n| n.checked_add(1))
                .ok_or(StoreError::Capacity)?;
            total = total
                .checked_add(body.len() as u64)
                .ok_or(StoreError::Capacity)?;
            if count > self.limits.max_blocks || total > self.limits.max_bytes {
                return Err(StoreError::Capacity);
            }
            let mut owned = Vec::new();
            owned
                .try_reserve_exact(body.len())
                .map_err(|_| StoreError::Allocation)?;
            owned.extend_from_slice(body);
            staged.try_reserve(1).map_err(|_| StoreError::Allocation)?;
            staged.push(Block { cid, body: owned });
        }
        self.blocks
            .try_reserve(staged.len())
            .map_err(|_| StoreError::Allocation)?;
        self.blocks.extend(staged);
        self.bytes = total;
        Ok(())
    }
}

impl MaterializedBlocks for MemoryBlockStore {
    fn get(&self, cid: DaslCid) -> Option<&[u8]> {
        self.blocks
            .iter()
            .find(|b| b.cid == cid)
            .map(|b| b.body.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_corrupt_or_over_capacity_batch_never_publishes_earlier_staging() {
        let a = [0xa0];
        let b = [0x80];
        let ca = DaslCid::for_block(raiv::format::DaslCodec::Drisl, &a);
        let cb = DaslCid::for_block(raiv::format::DaslCodec::Drisl, &b);
        let mut store = MemoryBlockStore::new(StoreLimits {
            max_blocks: 2,
            max_bytes: 2,
            wire: DecodeLimits::default(),
        });
        assert_eq!(
            store.stage([(ca, a.as_slice()), (cb, a.as_slice())].into_iter()),
            Err(StoreError::Integrity)
        );
        assert_eq!(store.block_count(), 0);
        assert_eq!(store.encoded_bytes(), 0);
        store.put(ca, &a).unwrap();
        let pointer = store.get(ca).unwrap().as_ptr();
        store.limits.max_blocks = 1;
        assert_eq!(
            store.stage([(cb, b.as_slice())].into_iter()),
            Err(StoreError::Capacity)
        );
        assert_eq!(store.block_count(), 1);
        assert_eq!(store.get(ca).unwrap().as_ptr(), pointer);
        store.limits.max_blocks = 2;
        store.limits.max_bytes = 1;
        assert_eq!(store.put(cb, &b), Err(StoreError::Capacity));
        assert!(store.get(cb).is_none());
        assert_eq!(store.encoded_bytes(), 1);
    }

    #[test]
    fn corrupted_existing_bytes_are_not_overwritten_or_hidden_by_idempotence() {
        let body = [0xa0];
        let cid = DaslCid::for_block(raiv::format::DaslCodec::Drisl, &body);
        let mut store = MemoryBlockStore::new(StoreLimits {
            max_blocks: 1,
            max_bytes: 1,
            wire: DecodeLimits::default(),
        });
        store.put(cid, &body).unwrap();
        store.blocks[0].body[0] = 0x80;
        assert_eq!(store.put(cid, &body), Err(StoreError::ConflictingBytes));
        assert_eq!(store.get(cid), Some(&[0x80][..]));
    }
}
