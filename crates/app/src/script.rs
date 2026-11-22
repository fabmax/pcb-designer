//! Scripted input for screenshots and manual testing (development aid):
//! `--script "key W; click 0,0; move 10.16,5.08; wait 5"`. Coordinates are schematic
//! world millimetres (`board` switches to the board tab and coordinates, `schematic` back;
//! `press x,y`, `move x,y`, `release x,y` drag, `double-click x,y`; `ui-click
//! x,y` clicks at a screen position in points, `ui-double-click x,y` / `ui-right-click x,y`
//! likewise, `ui-move x,y` hovers there, `type TEXT` types text, `run [ANALYSIS]` runs a
//! simulation and waits for it (`start [ANALYSIS]` doesn't wait), `zoom F` zooms the canvas under the pointer, `scroll DY` scrolls what's under it);
//! each step runs a few frames after the previous one.

use std::collections::VecDeque;
use view::ViewKind;

use egui::{Event, Modifiers, PointerButton, Pos2};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step2 {
    Move(Pos2),
    Press(Pos2),
    Release(Pos2),
    /// Two clicks in one frame (a double-click).
    Double(Pos2),
    /// Secondary button click (a context menu).
    Right(Pos2),
}

#[derive(Clone, Debug, PartialEq)]
enum Step {
    /// A key with modifiers; Cmd+X/C/V become egui's clipboard events (as egui-winit does).
    Key(crate::keymap::Shortcut),
    Text(String),
    Move([f64; 2]),
    Press([f64; 2]),
    Release([f64; 2]),
    /// Two clicks in one frame (a double-click).
    Double([f64; 2]),
    /// Screen position in points (for clicking widgets).
    Screen(Step2),
    Wait(u32),
    /// Following coordinates are in the board (true) or schematic (false) view.
    Board(bool),
    /// Run a simulation (the analysis named, or the panel's); later steps wait for it
    /// unless false (`start`).
    Run(String, bool),
    /// Zoom the canvas under the pointer by a factor (a pinch).
    Zoom(f32),
    /// Scroll what's under the pointer by points (positive: content moves up, as a
    /// downward scroll does).
    Scroll(f32),
    /// Hold modifiers for the following pointer steps (`hold Alt`; `hold` lets go).
    Hold(Modifiers),
    /// The answer to the next file dialog (`dialog PATH`; `dialog cancel`: none), given
    /// instead of showing it.
    Dialog(Option<std::path::PathBuf>),
}

#[derive(Debug, Default)]
pub struct Script {
    steps: VecDeque<Step>,
    /// Modifiers a `hold` step holds down (for the pointer events that follow).
    held: Modifiers,
    wait: u32,
    board: bool,
    /// The view changed (or the script just started): its tab is to be brought to the
    /// front, once, so clicks may still switch tabs afterwards.
    switched: bool,
    /// A `run` step asked for a simulation (the app takes it).
    run: Option<String>,
    /// Later steps wait for the simulation (`run`, not `start`).
    wait_run: bool,
    /// Answers for the next file dialogs (`dialog` steps).
    dialogs: VecDeque<Option<std::path::PathBuf>>,
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
                "key" => steps.push_back(Step::Key(arg.trim().parse().map_err(|e| format!("key {arg:?}: {e}"))?)),
                "move" => steps.push_back(Step::Move(point()?)),
                "click" => {
                    let p = point()?;
                    steps.extend([Step::Move(p), Step::Press(p), Step::Release(p)]);
                }
                "press" => steps.push_back(Step::Press(point()?)),
                "release" => steps.push_back(Step::Release(point()?)),
                "double-click" => {
                    let p = point()?;
                    steps.extend([Step::Move(p), Step::Double(p)]);
                }
                "ui-click" => {
                    let [x, y] = point()?;
                    let p = Pos2::new(x as f32, y as f32);
                    steps.extend([Step2::Move(p), Step2::Press(p), Step2::Release(p)].map(Step::Screen));
                }
                "ui-double-click" | "ui-right-click" => {
                    let [x, y] = point()?;
                    let p = Pos2::new(x as f32, y as f32);
                    let click = if verb == "ui-double-click" { Step2::Double(p) } else { Step2::Right(p) };
                    steps.extend([Step2::Move(p), click].map(Step::Screen));
                }
                "ui-press" | "ui-release" => {
                    let [x, y] = point()?;
                    let p = Pos2::new(x as f32, y as f32);
                    let step = if verb == "ui-press" { Step2::Press(p) } else { Step2::Release(p) };
                    steps.extend([Step2::Move(p), step].map(Step::Screen));
                }
                "type" => steps.push_back(Step::Text(arg.to_string())),
                "ui-move" => {
                    let [x, y] = point()?;
                    steps.push_back(Step::Screen(Step2::Move(Pos2::new(x as f32, y as f32))));
                }
                "run" => steps.push_back(Step::Run(arg.trim().to_string(), true)),
                "start" => steps.push_back(Step::Run(arg.trim().to_string(), false)),
                "board" => steps.push_back(Step::Board(true)),
                "schematic" => steps.push_back(Step::Board(false)),
                "scroll" => steps.push_back(Step::Scroll(arg.trim().parse().map_err(|e| format!("{cmd:?}: {e}"))?)),
                "zoom" => steps.push_back(Step::Zoom(arg.trim().parse().map_err(|e| format!("{cmd:?}: {e}"))?)),
                "hold" => {
                    let mut m = Modifiers::NONE;
                    for k in arg.split('+').map(str::trim).filter(|k| !k.is_empty()) {
                        match k.to_ascii_lowercase().as_str() {
                            "alt" | "option" => m.alt = true,
                            "shift" => m.shift = true,
                            "ctrl" => m.ctrl = true,
                            "cmd" => m |= Modifiers::COMMAND,
                            _ => return Err(format!("hold: unknown modifier {k:?}")),
                        }
                    }
                    steps.push_back(Step::Hold(m));
                }
                "wait" => steps.push_back(Step::Wait(arg.trim().parse().map_err(|e| format!("{cmd:?}: {e}"))?)),
                "dialog" => steps.push_back(Step::Dialog(match arg.trim() {
                    "cancel" => None,
                    "" => return Err("dialog: a path, or cancel".into()),
                    path => Some(path.into()),
                })),
                _ => return Err(format!("unknown script step {cmd:?}")),
            }
        }
        Ok(Script {
            steps,
            held: Modifiers::NONE,
            wait: 0,
            board: false,
            switched: true,
            run: None,
            wait_run: true,
            dialogs: VecDeque::new(),
        })
    }

    /// The answer a `dialog` step gave for the file dialog about to open, if any (`Some(None)`:
    /// cancelled).
    pub fn take_dialog(&mut self) -> Option<Option<std::path::PathBuf>> {
        self.dialogs.pop_front()
    }

    /// The simulation a `run` step asked for: an analysis name, empty for the panel's.
    pub fn take_run(&mut self) -> Option<String> {
        self.run.take()
    }

    /// Whether later steps wait for a running simulation.
    pub fn waits_for_run(&self) -> bool {
        self.wait_run
    }

    /// Leave the tab in front as it is at the start (`--show` opened one).
    pub fn keep_tab(&mut self) {
        self.switched = false;
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
        // A view that isn't laid out yet (just switched to) maps to no position: wait.
        if let Step::Move(p) | Step::Press(p) | Step::Release(p) | Step::Double(p) = step
            && !to_screen(p).is_finite()
        {
            self.steps.push_front(step);
            self.wait = 1;
            return Vec::new();
        }
        self.wait = GAP;
        let m = self.held;
        match step {
            Step::Hold(held) => {
                self.held = held;
                vec![Event::ModifiersChanged(held)]
            }
            Step::Key(s) => {
                let press = s.event();
                let mut release = press.clone();
                if let Event::Key { pressed, .. } = &mut release {
                    *pressed = false;
                }
                vec![press, release]
            }
            Step::Move(p) => vec![Event::PointerMoved(to_screen(p))],
            Step::Press(p) | Step::Release(p) => vec![Event::PointerButton {
                pos: to_screen(p),
                button: PointerButton::Primary,
                pressed: matches!(step, Step::Press(_)),
                modifiers: m,
            }],
            Step::Double(p) => [true, false, true, false]
                .map(|pressed| Event::PointerButton {
                    pos: to_screen(p),
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: m,
                })
                .to_vec(),
            Step::Text(t) => vec![Event::Text(t)],
            Step::Screen(Step2::Move(p)) => vec![Event::PointerMoved(p)],
            Step::Screen(Step2::Double(pos)) => [true, false, true, false]
                .map(|pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: m })
                .to_vec(),
            Step::Screen(Step2::Right(pos)) => [true, false]
                .map(|pressed| Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: m })
                .to_vec(),
            Step::Screen(Step2::Press(pos) | Step2::Release(pos)) => vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: matches!(step, Step::Screen(Step2::Press(_))),
                modifiers: m,
            }],
            Step::Zoom(f) => vec![Event::Zoom(f)],
            Step::Scroll(dy) => vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -dy),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: m,
            }],
            Step::Wait(n) => {
                self.wait = n;
                Vec::new()
            }
            Step::Dialog(answer) => {
                self.dialogs.push_back(answer);
                self.wait = 0;
                Vec::new()
            }
            Step::Run(name, wait) => {
                self.run = Some(name);
                self.wait_run = wait;
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
    fn dialog_steps_queue_answers() {
        let mut s = Script::parse("dialog /tmp/a.pcblib; dialog cancel; key Cmd+S").unwrap();
        assert!(Script::parse("dialog").is_err());
        assert_eq!(s.take_dialog(), None, "not reached yet");
        while !s.is_done() {
            s.events(|_, _| Pos2::ZERO);
        }
        assert_eq!(s.take_dialog(), Some(Some("/tmp/a.pcblib".into())));
        assert_eq!(s.take_dialog(), Some(None), "cancelled");
        assert_eq!(s.take_dialog(), None);
    }

    #[test]
    fn parse_steps() {
        let s = Script::parse(
            "key W; click 1.27,-2.54; wait 4; ui-move 10,20; type abc; ui-double-click 1,2; press 0,0; release 1,1; \
             double-click 2,2",
        )
        .unwrap();
        assert_eq!(s.steps.len(), 14);
        assert!(Script::parse("jump 1,2").is_err());
        assert!(Script::parse("click 1").is_err());
        assert!(Script::parse("key NoSuchKey").is_err());
    }
}
