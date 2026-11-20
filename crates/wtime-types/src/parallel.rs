//! Order-preserving parallel maps over independent items.

use std::sync::atomic::{AtomicUsize, Ordering};

/// `items.iter().map(|item| f(&mut state, item))` with up to `threads`
/// threads, each with its own `state` from `init`, and results in the order
/// of `items`. Items are handed out in order in small batches; the calling
/// thread is one of the workers, and a thread that cannot be spawned leaves
/// its share to the others, so the map completes on the calling thread
/// alone if need be. A panic in a worker propagates.
pub fn map_ordered<T, S, R>(
    items: &[T],
    threads: usize,
    init: impl Fn() -> S + Sync,
    f: impl Fn(&mut S, &T) -> R + Sync,
) -> Vec<R>
where
    T: Sync,
    R: Send,
{
    let threads = threads.clamp(1, items.len().max(1));
    if threads == 1 {
        let mut state = init();
        return items.iter().map(|item| f(&mut state, item)).collect();
    }
    let batch = (items.len() / (threads * 16)).clamp(1, 64);
    let next = AtomicUsize::new(0);
    let work = || {
        let mut state = init();
        let mut done = Vec::new();
        loop {
            let start = next.fetch_add(batch, Ordering::Relaxed);
            if start >= items.len() {
                break;
            }
            for (k, item) in items[start..(start + batch).min(items.len())].iter().enumerate() {
                done.push((start + k, f(&mut state, item)));
            }
        }
        done
    };
    let mut parts = std::thread::scope(|scope| {
        let spawned: Vec<_> = (1..threads)
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(scope, work).ok())
            .collect();
        let mut parts = vec![work()];
        for handle in spawned {
            match handle.join() {
                Ok(part) => parts.push(part),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        parts
    });
    let mut slots: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
    for part in parts.drain(..) {
        for (index, result) in part {
            slots[index] = Some(result);
        }
    }
    slots.into_iter().map(|slot| slot.expect("every item is mapped once")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_state_per_thread() {
        let items: Vec<u32> = (0..10_000).collect();
        for threads in [1, 2, 3, 8] {
            let out = map_ordered(&items, threads, || 0u32, |seen, x| {
                *seen += 1;
                x * 2
            });
            assert_eq!(out, items.iter().map(|x| x * 2).collect::<Vec<_>>());
        }
        assert!(map_ordered(&[] as &[u32], 4, || (), |_, x| *x).is_empty());
    }
}
