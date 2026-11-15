//! Keyboard movement of the selection in a flat list (KiCad import window).

/// A key that moves a list's selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKey {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

/// The row selected after `key` in a list of `len` rows showing `page` rows at a time.
/// Without a selection, Down/Home/PageDown start at the first row, Up/End/PageUp at the
/// last; moves stop at the ends.
pub fn step(len: usize, current: Option<usize>, key: ListKey, page: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len - 1;
    let page = page.max(1);
    Some(match (key, current.map(|c| c.min(last))) {
        (ListKey::Home, _) => 0,
        (ListKey::End, _) => last,
        (ListKey::Down | ListKey::PageDown, None) => 0,
        (ListKey::Up | ListKey::PageUp, None) => last,
        (ListKey::Down, Some(c)) => (c + 1).min(last),
        (ListKey::Up, Some(c)) => c.saturating_sub(1),
        (ListKey::PageDown, Some(c)) => (c + page).min(last),
        (ListKey::PageUp, Some(c)) => c.saturating_sub(page),
    })
}

#[cfg(test)]
mod tests {
    use super::ListKey::*;
    use super::*;

    #[test]
    fn moves_and_stops_at_the_ends() {
        assert_eq!(step(0, None, Down, 10), None);
        assert_eq!(step(5, None, Down, 10), Some(0));
        assert_eq!(step(5, None, Up, 10), Some(4));
        assert_eq!(step(5, Some(0), Up, 10), Some(0));
        assert_eq!(step(5, Some(4), Down, 10), Some(4));
        assert_eq!(step(5, Some(2), Down, 10), Some(3));
        assert_eq!(step(5, Some(2), Home, 10), Some(0));
        assert_eq!(step(5, Some(2), End, 10), Some(4));
        assert_eq!(step(50, Some(2), PageDown, 10), Some(12));
        assert_eq!(step(50, Some(45), PageDown, 10), Some(49));
        assert_eq!(step(50, Some(5), PageUp, 10), Some(0));
        // A selection beyond a shrunk list starts from its end.
        assert_eq!(step(3, Some(9), Up, 10), Some(1));
    }
}
