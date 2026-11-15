//! Drops mouse button releases without a matching press before egui sees them.
//!
//! macOS (winit) delivers a click on an unfocused window as press, release, release. When
//! both releases land in one frame, egui counts the second as another click 0 s after the
//! first: a double-click, which zoomed the window when it hit our title strip.

use egui::{Event, NUM_POINTER_BUTTONS, RawInput};

#[derive(Default)]
pub struct PointerFilter {
    down: [bool; NUM_POINTER_BUTTONS],
}

impl PointerFilter {
    pub fn filter(&mut self, raw: &mut RawInput) {
        raw.events.retain(|e| match e {
            Event::PointerButton { button, pressed, .. } => {
                let down = &mut self.down[*button as usize];
                let keep = *pressed || *down;
                *down = *pressed;
                keep
            }
            _ => true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Modifiers, PointerButton, pos2};

    fn button(button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton { pos: pos2(10.0, 10.0), button, pressed, modifiers: Modifiers::NONE }
    }

    fn run(filter: &mut PointerFilter, events: Vec<Event>) -> Vec<Event> {
        let mut raw = RawInput { events, ..Default::default() };
        filter.filter(&mut raw);
        raw.events
    }

    #[test]
    fn a_doubled_release_is_one_click() {
        let mut f = PointerFilter::default();
        let (press, release) = (button(PointerButton::Primary, true), button(PointerButton::Primary, false));
        let out = run(&mut f, vec![press.clone(), release.clone(), release.clone()]);
        assert_eq!(out, vec![press.clone(), release.clone()]);
        // Also when the extra release comes a frame later.
        assert_eq!(run(&mut f, vec![press.clone()]), vec![press.clone()]);
        assert_eq!(run(&mut f, vec![release.clone()]), vec![release.clone()]);
        assert_eq!(run(&mut f, vec![release.clone()]), vec![]);
    }

    #[test]
    fn real_double_clicks_and_other_buttons_pass() {
        let mut f = PointerFilter::default();
        let (p, r) = (button(PointerButton::Primary, true), button(PointerButton::Primary, false));
        let (mp, mr) = (button(PointerButton::Middle, true), button(PointerButton::Middle, false));
        let events = vec![p.clone(), mp.clone(), r.clone(), p.clone(), mr.clone(), r.clone()];
        assert_eq!(run(&mut f, events.clone()), events);
    }
}
