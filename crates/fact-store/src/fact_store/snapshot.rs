#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Immutable snapshots (P10.5.1).
//!
//! [`FactSnapshot`] captures the canonical serialised form of a store's
//! facts. Because the underlying `FactsModel` stores every category id-sorted
//! and `serde_json` preserves that order, snapshots are **byte-identical for
//! identical inputs**. There are no timestamps and no randomness: the digest
//! is a fixed FNV-1a 64-bit hash of the canonical bytes.

use crate::engineering_facts::FactsModel;
use crate::fact_store::store::FactStore;
use serde::{Deserialize, Serialize};

/// An immutable, deterministic snapshot of a [`FactStore`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactSnapshot {
    bytes: Vec<u8>,
    digest: String,
}

impl FactSnapshot {
    /// Capture a store. The bytes are the canonical JSON of the frozen model.
    pub fn capture(store: &FactStore) -> Self {
        let model = store.collection().model();
        let bytes = serde_json::to_vec(model).expect("FactsModel always serialises");
        FactSnapshot {
            digest: hex_digest(&bytes),
            bytes,
        }
    }

    /// Rehydrate a snapshot from canonical bytes, validating the round-trip.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, serde_json::Error> {
        let _: FactsModel = serde_json::from_slice(&bytes)?;
        let digest = hex_digest(&bytes);
        Ok(FactSnapshot { bytes, digest })
    }

    /// The canonical serialised facts.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The deterministic content digest (hex FNV-1a 64).
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Deserialise the captured facts.
    pub fn model(&self) -> Result<FactsModel, serde_json::Error> {
        serde_json::from_slice(&self.bytes)
    }

    /// Rebuild an immutable store from the snapshot. Reconstructing runs the
    /// same deterministic build, so the rebuilt store equals the original.
    pub fn restore(&self) -> Result<FactStore, serde_json::Error> {
        let model: FactsModel = serde_json::from_slice(&self.bytes)?;
        Ok(FactStore::build(model))
    }
}

/// Fixed FNV-1a 64-bit hash over the canonical bytes. Deterministic: no
/// timestamps, no randomness, no platform dependence.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:016x}", fnv1a64(bytes))
}

#[cfg(test)]
mod digest_tests {
    use super::fnv1a64;

    #[test]
    fn fnv1a64_is_deterministic() {
        assert_eq!(fnv1a64(b""), fnv1a64(b""));
        assert_eq!(fnv1a64(b"fact-store"), fnv1a64(b"fact-store"));
        assert_ne!(fnv1a64(b"a"), fnv1a64(b"b"));
    }
}
