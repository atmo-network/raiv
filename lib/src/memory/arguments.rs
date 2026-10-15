//! Borrowed preflighted object-patch arguments and captured authorized bounds.

use super::{plane, push, Memory, PatchError};
use crate::{
    format::{validate_model_value, DecodeLimits, ValueView},
    limits::Limits,
};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkippedPlane<'a> {
    pub name: &'a str,
}

impl SkippedPlane<'_> {
    pub fn reason(&self) -> &'static str {
        "unsupported_root"
    }
}

/// Exact canonical bytes of the object patch, without payload ownership or paths.
/// Limits are captured; runtime admission and final result/context staging remain
/// separate. Provider JSON adapters must reject duplicates before CBOR conversion.
pub struct PatchArguments<'a> {
    bytes: &'a [u8],
    limits: Limits,
}

impl<'a> PatchArguments<'a> {
    pub fn decode(bytes: &'a [u8], limits: &Limits) -> Result<Self, PatchError> {
        let wire = DecodeLimits {
            max_block_bytes: limits.max_model_reply_bytes,
            max_depth: limits.max_depth,
            max_collection_entries: limits.max_collection_entries,
        };
        validate_model_value(bytes, &wire)?;
        let root = ValueView::preflighted(bytes);
        let map = root.map().map_err(|_| PatchError::InvalidArguments)?;
        let mut stack = Vec::new();
        let mut count = map.len();
        if count > limits.max_patch_entries {
            return Err(PatchError::EntryLimit);
        }
        for entry in map {
            let (key, value) = entry?;
            let selected = plane(key);
            if selected && !value.is_null() && value.map().is_err() {
                return Err(PatchError::PlaneNotMap);
            }
            push(&mut stack, (value, false, selected))?;
        }
        while let Some((value, in_array, selected)) = stack.pop() {
            if selected && value.is_null() && in_array {
                return Err(PatchError::NullInArray);
            }
            match value.encoded()[0] >> 5 {
                5 => {
                    let map = value.map()?;
                    count = count.checked_add(map.len()).ok_or(PatchError::EntryLimit)?;
                    if count > limits.max_patch_entries {
                        return Err(PatchError::EntryLimit);
                    }
                    for entry in map {
                        push(&mut stack, (entry?.1, in_array, selected))?;
                    }
                }
                4 => {
                    let list = value.list()?;
                    count = count
                        .checked_add(list.len())
                        .ok_or(PatchError::EntryLimit)?;
                    if count > limits.max_patch_entries {
                        return Err(PatchError::EntryLimit);
                    }
                    for child in list {
                        push(&mut stack, (child?, true, selected))?;
                    }
                }
                _ => {}
            }
        }
        Ok(Self {
            bytes,
            limits: *limits,
        })
    }

    /// Canonical input order, exact borrowed names; processed does not imply changed.
    pub fn processed_planes(&self) -> impl Iterator<Item = &'a str> {
        ValueView::preflighted(self.bytes)
            .map()
            .expect("validated root")
            .map(|e| e.expect("validated field").0)
            .filter(|key| plane(key))
    }

    pub fn skipped_planes(&self) -> impl Iterator<Item = SkippedPlane<'a>> {
        ValueView::preflighted(self.bytes)
            .map()
            .expect("validated root")
            .map(|e| e.expect("validated field").0)
            .filter(|key| !plane(key))
            .map(|name| SkippedPlane { name })
    }

    /// Bounded transient report; not a Memory block, capability or applied-byte rewrite.
    /// Result/context caps bound generated output; model-reply caps bound incoming arguments.
    /// Both states are typed DATA. A runtime must stage this report/context before assignment.
    pub fn report(
        &self,
        before: &Memory,
        accepted: &Memory,
    ) -> Result<crate::format::ModelValue, PatchError> {
        let wire = DecodeLimits {
            max_block_bytes: self
                .limits
                .max_context_bytes
                .min(self.limits.max_block_bytes),
            max_depth: self.limits.max_depth,
            max_collection_entries: self.limits.max_collection_entries,
        };
        let mut e = crate::format::RecordEncoder::new(&wire)?;
        e.map(4)?;
        e.text("memory")?;
        e.encoded(accepted.value_bytes())?;
        e.text("changed")?;
        e.boolean(before != accepted)?;
        e.text("skipped_planes")?;
        e.list(self.skipped_planes().count() as u64)?;
        for skipped in self.skipped_planes() {
            e.map(2)?;
            e.text("plane")?;
            e.text(skipped.name)?;
            e.text("reason")?;
            e.text(skipped.reason())?;
        }
        e.text("processed_planes")?;
        e.list(self.processed_planes().count() as u64)?;
        for name in self.processed_planes() {
            e.text(name)?;
        }
        Ok(crate::format::ModelValue::from_encoded(e.finish(), &wire)?)
    }

    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Build the sole candidate buffer; runtime validates full result/trajectory/context
    /// before RAM assignment. No source copy or mutation occurs during planning.
    pub fn patched(&self, memory: &Memory) -> Result<Memory, PatchError> {
        memory.patched(ValueView::preflighted(self.bytes), &self.limits)
    }

    pub fn apply<'memory>(&self, memory: &'memory mut Memory) -> Result<&'memory [u8], PatchError> {
        let candidate = self.patched(memory)?;
        // Preserve even the original buffer on pure no-effect acceptance.
        if candidate != *memory {
            *memory = candidate;
        }
        Ok(memory.value_bytes())
    }
}

#[cfg(test)]
mod tests;
