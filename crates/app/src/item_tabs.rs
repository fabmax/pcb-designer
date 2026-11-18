//! Which library item tabs to close when there are more than the maximum.

/// What closing picks by.
#[derive(Clone, Copy, Debug)]
pub struct TabInfo {
    pub id: u64,
    /// When it was last focused (larger is later).
    pub used: u64,
    pub preview: bool,
    /// In front of its tab group (seen in a split).
    pub visible: bool,
}

/// The tabs to close so that at most `max` remain, never one of `keep`: hidden ones
/// before visible ones, the least recently used first, the preview tab last.
pub fn to_close(tabs: &[TabInfo], max: usize, keep: &[u64]) -> Vec<u64> {
    let excess = tabs.len().saturating_sub(max);
    let mut candidates: Vec<&TabInfo> = tabs.iter().filter(|t| !keep.contains(&t.id)).collect();
    candidates.sort_by_key(|t| (t.preview, t.visible, t.used));
    candidates.into_iter().take(excess).map(|t| t.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: u64, used: u64) -> TabInfo {
        TabInfo { id, used, preview: false, visible: false }
    }

    #[test]
    fn closes_the_least_recently_used() {
        let tabs = [tab(1, 30), tab(2, 10), tab(3, 20)];
        assert_eq!(to_close(&tabs, 3, &[]), Vec::<u64>::new());
        assert_eq!(to_close(&tabs, 2, &[]), vec![2]);
        assert_eq!(to_close(&tabs, 1, &[]), vec![2, 3]);
    }

    #[test]
    fn keeps_visible_preview_and_kept_tabs_longest() {
        let tabs =
            [TabInfo { visible: true, ..tab(1, 1) }, TabInfo { preview: true, ..tab(2, 2) }, tab(3, 3), tab(4, 4)];
        assert_eq!(to_close(&tabs, 3, &[3]), vec![4], "kept");
        assert_eq!(to_close(&tabs, 2, &[]), vec![3, 4], "hidden first");
        assert_eq!(to_close(&tabs, 1, &[]), vec![3, 4, 1], "visible before the preview");
        assert_eq!(to_close(&tabs, 0, &[1, 2, 3, 4]), Vec::<u64>::new(), "never below the kept ones");
    }
}
