use std::sync::atomic::{AtomicU64, Ordering};

// Preserve checked, unique allocation on both the host MSRV and newer toolchains.
pub(crate) fn next_u64(source: &AtomicU64) -> Result<u64, ()> {
    let mut current = source.load(Ordering::Relaxed);
    loop {
        let next = current.checked_add(1).ok_or(())?;
        match source.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Ok(current),
            Err(observed) => current = observed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_allocation_does_not_wrap() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(next_u64(&counter), Ok(u64::MAX - 1));
        assert_eq!(next_u64(&counter), Err(()));
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }
}
