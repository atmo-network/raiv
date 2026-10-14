//! Private iterative object merge and empty-object normalization over borrowed byte spans.
//! The flat plan owns only bounded metadata, never a recursive runtime value tree.

use super::{try_push, CodecError, DecodeLimits, RecordEncoder, ValueView};
use alloc::vec::Vec;
use core::ops::Range;

struct Node<'a> {
    base: Option<ValueView<'a>>,
    patch: Option<ValueView<'a>>,
    children: Range<usize>,
    count: usize,
    major: u8,
}
struct Edge<'a> {
    key: Option<&'a str>,
    node: usize,
}
impl<'a> Node<'a> {
    fn new(base: Option<ValueView<'a>>, patch: Option<ValueView<'a>>) -> Self {
        Self {
            base,
            patch,
            children: 0..0,
            count: 0,
            major: 0,
        }
    }
    fn value(&self) -> ValueView<'a> {
        self.patch
            .or(self.base)
            .expect("every plan node has a source")
    }
    fn empty_object(&self) -> bool {
        self.major == 5 && self.count == 0
    }
}

/// Preflighted source and patch are bounded independently by their domain owner.
/// Every plan node consumes a distinct source/patch value; metadata is bounded by
/// their combined encoded lengths. One bounded output buffer contains the whole record.
pub(crate) fn merge_map_field(
    record: ValueView<'_>,
    field: &str,
    patch: ValueView<'_>,
    limits: &DecodeLimits,
    accept_root_key: fn(&str) -> bool,
) -> Result<Vec<u8>, CodecError> {
    let base = record
        .map()?
        .find_map(|entry| match entry {
            Ok((key, value)) if key == field => Some(value),
            _ => None,
        })
        .ok_or(CodecError::Malformed)?;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    try_push(&mut nodes, Node::new(Some(base), Some(patch)))?;
    let mut index = 0;
    while index < nodes.len() {
        let value = nodes[index].value();
        let major = value.encoded()[0] >> 5;
        let start = edges.len();
        if major == 5 {
            // Canonical maps admit a linear two-stream union, without key sorting/copies.
            let mut old = nodes[index]
                .base
                .filter(|v| v.encoded()[0] >> 5 == 5)
                .map(ValueView::map)
                .transpose()?
                .into_iter()
                .flatten()
                .peekable();
            let mut new = nodes[index]
                .patch
                .map(ValueView::map)
                .transpose()?
                .into_iter()
                .flatten()
                .filter(|entry| {
                    index != 0 || entry.as_ref().map_or(true, |(key, _)| accept_root_key(key))
                })
                .peekable();
            while old.peek().is_some() || new.peek().is_some() {
                let order = match (old.peek(), new.peek()) {
                    (Some(Ok((a, _))), Some(Ok((b, _)))) => (a.len(), *a).cmp(&(b.len(), *b)),
                    (Some(Err(error)), _) | (_, Some(Err(error))) => return Err(*error),
                    (Some(_), None) => core::cmp::Ordering::Less,
                    (None, Some(_)) => core::cmp::Ordering::Greater,
                    (None, None) => unreachable!(),
                };
                let (key, base, patch) = match order {
                    core::cmp::Ordering::Less => {
                        let (key, value) = old.next().expect("peeked entry")?;
                        (key, Some(value), None)
                    }
                    core::cmp::Ordering::Greater => {
                        let (key, value) = new.next().expect("peeked entry")?;
                        (key, None, Some(value))
                    }
                    core::cmp::Ordering::Equal => {
                        let (_, base) = old.next().expect("peeked entry")?;
                        let (key, patch) = new.next().expect("peeked entry")?;
                        (key, Some(base), Some(patch))
                    }
                };
                if patch.is_some_and(ValueView::is_null) {
                    continue;
                }
                let child = nodes.len();
                try_push(&mut nodes, Node::new(base, patch))?;
                try_push(
                    &mut edges,
                    Edge {
                        key: Some(key),
                        node: child,
                    },
                )?;
            }
        } else if major == 4 {
            // Replacement arrays are data: their objects normalize, but never merge.
            for child in value.list()? {
                let child = child?;
                let node = nodes.len();
                try_push(&mut nodes, Node::new(Some(child), None))?;
                try_push(&mut edges, Edge { key: None, node })?;
            }
        }
        nodes[index].major = major;
        nodes[index].children = start..edges.len();
        index += 1;
    }
    // Children always have higher indices; normalize bottom-up without recursion.
    for index in (0..nodes.len()).rev() {
        nodes[index].count = edges[nodes[index].children.clone()]
            .iter()
            .filter(|edge| nodes[index].major != 5 || !nodes[edge.node].empty_object())
            .count();
    }
    let mut writer = RecordEncoder::new(limits)?;
    writer.map(record.map()?.len())?;
    for entry in record.map()? {
        let (key, value) = entry?;
        writer.text(key)?;
        if key != field {
            writer.encoded(value.encoded())?;
            continue;
        }
        enum Write<'a> {
            Node(usize),
            Key(&'a str),
        }
        let mut stack = Vec::new();
        try_push(&mut stack, Write::Node(0))?;
        while let Some(task) = stack.pop() {
            let index = match task {
                Write::Key(key) => {
                    writer.text(key)?;
                    continue;
                }
                Write::Node(index) => index,
            };
            let node = &nodes[index];
            match node.major {
                5 => writer.map(node.count as u64)?,
                4 => writer.list(node.count as u64)?,
                _ => {
                    writer.encoded(node.value().encoded())?;
                    continue;
                }
            }
            for edge in edges[node.children.clone()].iter().rev() {
                if node.major == 5 && nodes[edge.node].empty_object() {
                    continue;
                }
                try_push(&mut stack, Write::Node(edge.node))?;
                if let Some(key) = edge.key {
                    try_push(&mut stack, Write::Key(key))?;
                }
            }
        }
    }
    Ok(writer.finish())
}
