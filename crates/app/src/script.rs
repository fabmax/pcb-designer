//! Scripted input for screenshots and manual testing (development aid):
//! `--script "key W; click 0,0; move 10.16,5.08; wait 5"`. Coordinates are schematic
//! world millimetres (`board` switches to the board tab and coordinates, `schematic` back; `ui-click
//! x,y` clicks at a screen position in points, `ui-move x,y` hovers there); each step runs a few frames after the previous one.

use std::collections::VecDeque;
use view::ViewKind;

use egui::{Event, Key, Modifiers, PointerButton, Pos2};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step2 {
    Move(Pos2),
    Press(Pos2),
    Release(Pos2),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Key(Key),
    Move([f64; 2]),
    Press([f64; 2]),
    Release([f64; 2]),
    /// Screen position in points (for clicking widgets).
    Screen(Step2),
    Wait(u32),
    /// Following coordinates are in the board (true) or schematic (false) view.
    Board(bool),
}

#[derive(Debug, Default)]
pub struct Script {
    steps: VecDeque<Step>,
    wait: u32,
    board: bool,
    /// The view changed (or the script just started): its tab is to be brought to the
    /// front, once, so clicks may still switch tabs afterwards.
    switched: bool,
}

/// Frames between steps (lets the camera fit and the UI settle).
const GAP: u32 = 3;

impl Script {
    /// The view the script currently works in.
    pub fn view(&self) -> ViewKind {
        if self.board { ViewKind::Board } else { ViewKind::Schematic }
    }

    /// The view to bring to the front, at the start and after `board` / `schematic`.
    pub fn take_switch(&mut self) -> Option<ViewKind> {
        std::mem::take(&mut self.switched).then(|| self.view())
    }

    pub fn parse(text: &str) -> Result<Script, String> {
        let mut steps = VecDeque::from([Step::Wait(20)]);
        for cmd in text.split(';').map(str::trim).filter(|c| !c.is_empty()) {
            let (verb, arg) = cmd.split_once(' ').unwrap_or((cmd, ""));
            let point = || -> Result<[f64; 2], String> {
                let (x, y) = arg.split_once(',').ok_or(format!("expected x,y in {cmd:?}"))?;
                let num = |v: &str| v.trim().parse::<f64>().map_err(|e| format!("{cmd:?}: {e}"));
                Ok([num(x)?, num(y)?])
            };
            match verb {
                "key" => steps.push_back(Step::Key(Key::from_name(arg.trim()).ok_or(format!("unknown key {arg:?}"))?)),
                "move" => steps.push_back(Step::Move(point()?)),
                "click" => {
                    let p = point()?;
                    steps.extend([Step::Move(p), Step::Press(p), Step::Release(p)]);
                }
                "ui-click" => {
                    let [x, y] = point()?;
                    let p = Pos2::new(x as f32, y as f32);
                    steps.extend([Step2::Move(p), Step2::Press(p), Step2::Release(p)].map(Step::Screen));
                }
                "ui-move" => {
                    let [x, y] = point()?;
                    steps.push_back(Step::Screen(Step2::Move(Pos2::new(x as f32, y as f32))));
                }
                "board" => steps.push_back(Step::Board(true)),
                "schematic" => steps.push_back(Step::Board(false)),
                "wait" => steps.push_back(Step::Wait(arg.trim().parse().map_err(|e| format!("{cmd:?}: {e}"))?)),
                _ => return Err(format!("unknown script step {cmd:?}")),
            }
        }
        Ok(Script { steps, wait: 0, board: false, switched: true })
    }

    pub fn is_done(&self) -> bool {
        self.steps.is_empty() && self.wait == 0
    }

    /// Input events for this frame.
    /// `to_screen(board, world)` maps world millimetres of a view to screen points.
    pub fn events(&mut self, to_screen: impl Fn(bool, [f64; 2]) -> Pos2) -> Vec<Event> {
        let board = self.board;
        let to_screen = |p| to_screen(board, p);
        if self.wait > 0 {
            self.wait -= 1;
            return Vec::new();
        }
        let Some(step) = self.steps.pop_front() else { return Vec::new() };
        self.wait = GAP;
        let m = Modifiers::NONE;
        match step {
            Step::Key(key) => vec![
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m },
                Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: m },
            ],
            Step::Move(p) => vec![Event::PointerMoved(to_screen(p))],
            Step::Press(p) | Step::Release(p) => vec![Event::PointerButton {
                pos: to_screen(p),
                button: PointerButton::Primary,
                pressed: matches!(step, Step::Press(_)),
                modifiers: m,
            }],
            Step::Screen(Step2::Move(p)) => vec![Event::PointerMoved(p)],
            Step::Screen(Step2::Press(pos) | Step2::Release(pos)) => vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: matches!(step, Step::Screen(Step2::Press(_))),
                modifiers: m,
            }],
            Step::Wait(n) => {
                self.wait = n;
                Vec::new()
            }
            Step::Board(b) => {
                self.board = b;
                self.switched = true;
                self.wait = 0;
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_steps() {
        let s = Script::parse("key W; click 1.27,-2.54; wait 4; ui-move 10,20").unwrap();
        assert_eq!(s.steps.len(), 7);
        assert!(Script::parse("jump 1,2").is_err());
        assert!(Script::parse("click 1").is_err());
        assert!(Script::parse("key NoSuchKey").is_err());
    }
}
