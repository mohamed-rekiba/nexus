use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use nexus_api::Order;

/// Sorts items by their [`Order`]: `before` / `after` rules first, then higher `priority`, then
/// id. Rules that name unknown ids are ignored. Items caught in a rule cycle go last, by
/// priority and id.
pub(crate) fn sort_by_order<T>(items: Vec<(String, Order, T)>) -> Vec<T> {
    let index: BTreeMap<String, usize> = items.iter().enumerate().map(|(i, (id, _, _))| (id.clone(), i)).collect();
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); items.len()];
    let mut incoming = vec![0usize; items.len()];
    for (i, (_, order, _)) in items.iter().enumerate() {
        let targets = order.before.iter().filter_map(|b| index.get(b)).map(|&j| (i, j));
        let sources = order.after.iter().filter_map(|a| index.get(a)).map(|&j| (j, i));
        for (from, to) in targets.chain(sources) {
            if from != to {
                edges[from].push(to);
                incoming[to] += 1;
            }
        }
    }

    let key = |i: usize| (items[i].1.priority, Reverse(items[i].0.clone()), i);
    let mut ready: BinaryHeap<_> = (0..items.len()).filter(|&i| incoming[i] == 0).map(key).collect();
    let mut sorted = Vec::with_capacity(items.len());
    while let Some((_, _, i)) = ready.pop() {
        sorted.push(i);
        for &next in &edges[i] {
            incoming[next] -= 1;
            if incoming[next] == 0 {
                ready.push(key(next));
            }
        }
    }
    if sorted.len() < items.len() {
        let mut stuck: Vec<_> = (0..items.len()).filter(|i| !sorted.contains(i)).map(key).collect();
        tracing::warn!(count = stuck.len(), "before/after rules form a cycle; ordering those items by priority");
        stuck.sort_by(|a, b| b.cmp(a));
        sorted.extend(stuck.into_iter().map(|(_, _, i)| i));
    }

    let mut slots: Vec<Option<T>> = items.into_iter().map(|(_, _, t)| Some(t)).collect();
    sorted.into_iter().map(|i| slots[i].take().expect("each index once")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(items: Vec<(&str, Order)>) -> Vec<String> {
        sort_by_order(items.into_iter().map(|(id, o)| (id.to_owned(), o, id.to_owned())).collect())
    }

    #[test]
    fn higher_priority_goes_first_and_ids_break_ties() {
        let sorted = ids(vec![("b", Order::default()), ("a", Order::default()), ("high", Order::priority(10))]);
        assert_eq!(sorted, ["high", "a", "b"]);
    }

    #[test]
    fn before_and_after_win_over_priority() {
        assert_eq!(ids(vec![("x", Order::priority(100).after("y")), ("y", Order::default())]), ["y", "x"]);
        assert_eq!(ids(vec![("x", Order::priority(100)), ("y", Order::default().before("x"))]), ["y", "x"]);
        assert_eq!(ids(vec![("x", Order::default().after("missing"))]), ["x"]);
    }

    #[test]
    fn a_rule_cycle_still_returns_every_item() {
        let sorted = ids(vec![
            ("x", Order::default().before("y")),
            ("y", Order::default().before("x")),
            ("z", Order::default()),
        ]);
        assert_eq!(sorted, ["z", "x", "y"]);
    }
}
