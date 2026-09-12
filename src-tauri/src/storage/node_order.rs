use std::collections::HashSet;

/// Only replace slots belonging to the visible subset. Hidden nodes retain
/// their relative position; stale IDs are discarded and new IDs appended.
pub fn merge_order(
    saved: &[String],
    all: &[String],
    requested: &[String],
) -> Result<Vec<String>, String> {
    let valid: HashSet<&String> = all.iter().collect();
    let mut seen = HashSet::new();
    if requested
        .iter()
        .any(|id| !valid.contains(id) || !seen.insert(id))
    {
        return Err("节点列表已变化，请刷新后重新排序".into());
    }
    let selected = seen;
    let mut emitted = HashSet::new();
    let base: Vec<String> = saved
        .iter()
        .chain(all)
        .filter(|id| valid.contains(id) && emitted.insert(*id))
        .cloned()
        .collect();
    let mut replacements = requested.iter();
    Ok(base
        .into_iter()
        .map(|id| {
            if selected.contains(&id) {
                replacements.next().expect("same subset size").clone()
            } else {
                id
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ids(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }
    #[test]
    fn preserves_hidden_slots_and_refresh_order() {
        assert_eq!(
            merge_order(&[], &ids("a b c d"), &ids("d b")).unwrap(),
            ids("a d c b")
        );
        assert_eq!(
            merge_order(&ids("d b a stale"), &ids("a b c d new"), &[]).unwrap(),
            ids("d b a c new")
        );
        assert_eq!(
            merge_order(&ids("d b a c"), &ids("a b c d"), &ids("a b c d")).unwrap(),
            ids("a b c d")
        );
    }
    #[test]
    fn rejects_invalid_and_duplicate_ids() {
        assert!(merge_order(&[], &ids("a b"), &ids("a a")).is_err());
        assert!(merge_order(&[], &ids("a b"), &ids("gone")).is_err());
    }
}
