//! A small ordered parallel map over scoped threads. Each worker gets its own state (the style
//! engine's cascades are not `Sync`, so every worker builds its own).

use std::sync::atomic::{AtomicUsize, Ordering};

/// Worker count: `MOCHI_TOKENS_THREADS` (1 = sequential), else the available parallelism, at
/// most 8.
pub fn threads() -> usize {
    std::env::var("MOCHI_TOKENS_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n >= 1)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map_or(1, std::num::NonZeroUsize::get)
                .min(8)
        })
}

/// `items.map(|i| work(&mut state, i))` in input order; `init` builds one state per worker.
pub fn map_with<T, S, R>(
    items: &[T],
    init: impl Fn() -> S + Sync,
    work: impl Fn(&mut S, &T) -> R + Sync,
) -> Vec<R>
where
    T: Sync,
    R: Send,
{
    let workers = threads().min(items.len()).max(1);
    if workers == 1 {
        let mut state = init();
        return items.iter().map(|i| work(&mut state, i)).collect();
    }
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, R)> = Vec::with_capacity(items.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut state = init();
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        out.push((i, work(&mut state, item)));
                    }
                    out
                })
            })
            .collect();
        for h in handles {
            match h.join() {
                Ok(part) => done.extend(part),
                Err(e) => std::panic::resume_unwind(e),
            }
        }
    });
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_come_back_in_input_order_with_one_state_per_worker() {
        let items: Vec<usize> = (0..200).collect();
        let out = map_with(
            &items,
            || 0usize,
            |n, i| {
                *n += 1;
                i * 2
            },
        );
        assert_eq!(out, (0..200).map(|i| i * 2).collect::<Vec<_>>());
        assert!(map_with(&[] as &[usize], || (), |(), i| *i).is_empty());
    }
}
