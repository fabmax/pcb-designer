//! Tool windows, IntelliJ style: a narrow bar of icon buttons on the left and right
//! window edges, one button per panel. Each bar has a top and a bottom group; the left
//! bar also holds the bottom area's group at its foot. A group shows at most one panel;
//! clicking its button again hides it. When both groups of a side show a panel, they
//! stack (top over bottom) with a draggable split. Buttons are dragged to reorder them or
//! to move a panel to another group. Panels never open or switch on their own.
//!
//! [`Dock`] is the saved state (the groups, which panel each shows, the sizes). [`Unseen`]
//! counts problems that appeared while the Messages panel was hidden (its badge).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::icons::Icon;
use crate::keymap::Action;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Panel {
    Project,
    Messages,
    Properties,
    ToolSettings,
    BoardSetup,
    SchematicSetup,
    Overview,
    Nets,
    Parts,
    Activity,
}

/// A group of panel buttons and the place its panel shows in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    LeftTop,
    LeftBottom,
    RightTop,
    RightBottom,
    Bottom,
}

/// A window edge with panels: left and right (each two stacked groups), bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Bottom,
}

impl Group {
    pub const ALL: [Group; 5] = [Group::LeftTop, Group::LeftBottom, Group::RightTop, Group::RightBottom, Group::Bottom];

    /// The groups on a side, top first.
    pub fn on(side: Side) -> &'static [Group] {
        match side {
            Side::Left => &[Group::LeftTop, Group::LeftBottom],
            Side::Right => &[Group::RightTop, Group::RightBottom],
            Side::Bottom => &[Group::Bottom],
        }
    }
}

impl Panel {
    pub const ALL: [Panel; 10] = [
        Panel::Project,
        Panel::Messages,
        Panel::Properties,
        Panel::ToolSettings,
        Panel::BoardSetup,
        Panel::SchematicSetup,
        Panel::Overview,
        Panel::Nets,
        Panel::Parts,
        Panel::Activity,
    ];

    /// Where the panel is by default (and after Reset layout).
    pub fn default_group(self) -> Group {
        match self {
            Panel::Project => Group::LeftTop,
            Panel::Messages | Panel::Activity => Group::Bottom,
            Panel::Properties | Panel::ToolSettings | Panel::BoardSetup | Panel::SchematicSetup | Panel::Overview => {
                Group::RightTop
            }
            Panel::Nets | Panel::Parts => Group::RightBottom,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Panel::Project => "Project",
            Panel::Messages => "Messages",
            Panel::Properties => "Properties",
            Panel::ToolSettings => "Tool settings",
            Panel::BoardSetup => "Board setup",
            Panel::SchematicSetup => "Schematic setup",
            Panel::Overview => "Overview",
            Panel::Nets => "Nets",
            Panel::Parts => "Parts",
            Panel::Activity => "Agent activity",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Panel::Project => Icon::Tree,
            Panel::Messages => Icon::Messages,
            Panel::Properties => Icon::Properties,
            Panel::ToolSettings => Icon::Wrench,
            Panel::BoardSetup => Icon::BoardSetup,
            Panel::SchematicSetup => Icon::SchematicSetup,
            Panel::Overview => Icon::Info,
            Panel::Nets => Icon::Nets,
            Panel::Parts => Icon::Part,
            Panel::Activity => Icon::Activity,
        }
    }

    /// The keyboard action showing or hiding it.
    pub fn action(self) -> Action {
        match self {
            Panel::Project => Action::PanelProject,
            Panel::Messages => Action::PanelMessages,
            Panel::Properties => Action::PanelProperties,
            Panel::ToolSettings => Action::PanelToolSettings,
            Panel::BoardSetup => Action::PanelBoardSetup,
            Panel::SchematicSetup => Action::PanelSchematicSetup,
            Panel::Overview => Action::PanelOverview,
            Panel::Nets => Action::PanelNets,
            Panel::Parts => Action::PanelParts,
            Panel::Activity => Action::PanelActivity,
        }
    }

    pub fn of_action(action: Action) -> Option<Panel> {
        Panel::ALL.into_iter().find(|p| p.action() == action)
    }

    /// By its saved name (`board_setup`), for `--panel`.
    pub fn parse(name: &str) -> Option<Panel> {
        serde_json::from_value(serde_json::Value::String(name.replace('-', "_"))).ok()
    }
}

/// The panel groups, which panel each shows, and the sizes (points). Saved in the
/// settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dock {
    /// The panels of each group in bar order. Panels missing from every group (added in
    /// a later version) go to their default group.
    pub groups: BTreeMap<Group, Vec<Panel>>,
    /// The panel each group shows.
    pub open: BTreeMap<Group, Panel>,
    pub left_width: f32,
    pub right_width: f32,
    pub bottom_height: f32,
    /// The upper panel's share of a side's height when two panels stack.
    pub left_split: f32,
    pub right_split: f32,
}

impl Default for Dock {
    fn default() -> Self {
        let mut groups: BTreeMap<Group, Vec<Panel>> = Group::ALL.iter().map(|&g| (g, Vec::new())).collect();
        for p in Panel::ALL {
            groups.entry(p.default_group()).or_default().push(p);
        }
        Dock {
            groups,
            open: BTreeMap::from([(Group::LeftTop, Panel::Project), (Group::RightTop, Panel::Properties)]),
            left_width: 240.0,
            right_width: 300.0,
            bottom_height: 200.0,
            left_split: 0.5,
            right_split: 0.5,
        }
    }
}

impl Dock {
    /// Every panel in exactly one group (after loading a file from another version or a
    /// hand edit); open panels in their group.
    pub fn normalize(&mut self) {
        let mut seen = HashSet::new();
        for g in Group::ALL {
            let list = self.groups.entry(g).or_default();
            list.retain(|p| seen.insert(*p));
        }
        for p in Panel::ALL {
            if !seen.contains(&p) {
                self.groups.entry(p.default_group()).or_default().push(p);
            }
        }
        let groups = &self.groups;
        self.open.retain(|g, p| groups.get(g).is_some_and(|l| l.contains(p)));
        for split in [&mut self.left_split, &mut self.right_split] {
            if !split.is_finite() {
                *split = 0.5;
            }
            *split = split.clamp(0.15, 0.85);
        }
    }

    /// The panels of `group`, in bar order.
    pub fn panels(&self, group: Group) -> &[Panel] {
        self.groups.get(&group).map_or(&[], |v| v.as_slice())
    }

    /// The group `panel` is in.
    pub fn group_of(&self, panel: Panel) -> Group {
        Group::ALL.into_iter().find(|g| self.panels(*g).contains(&panel)).unwrap_or(panel.default_group())
    }

    /// The panel `group` shows.
    pub fn open(&self, group: Group) -> Option<Panel> {
        self.open.get(&group).copied()
    }

    pub fn is_open(&self, panel: Panel) -> bool {
        self.open(self.group_of(panel)) == Some(panel)
    }

    /// Show `panel` (replacing its group's panel), or hide it if it is showing.
    pub fn toggle(&mut self, panel: Panel) {
        if self.is_open(panel) { self.hide(self.group_of(panel)) } else { self.show(panel) }
    }

    pub fn show(&mut self, panel: Panel) {
        self.open.insert(self.group_of(panel), panel);
    }

    pub fn hide(&mut self, group: Group) {
        self.open.remove(&group);
    }

    /// Put `panel` into `group` before the panel at `index` (of the group as it is now).
    /// An open panel stays open in its new group.
    pub fn move_panel(&mut self, panel: Panel, group: Group, index: usize) {
        let from = self.group_of(panel);
        let was_open = self.is_open(panel);
        let list = self.groups.entry(from).or_default();
        let Some(at) = list.iter().position(|p| *p == panel) else { return };
        list.remove(at);
        let index = if from == group && at < index { index - 1 } else { index };
        let list = self.groups.entry(group).or_default();
        list.insert(index.min(list.len()), panel);
        if was_open {
            self.open.remove(&from);
            self.open.insert(group, panel);
        }
    }

    /// The panels a side shows, top first.
    pub fn shown(&self, side: Side) -> Vec<(Group, Panel)> {
        Group::on(side).iter().filter_map(|&g| Some((g, self.open(g)?))).collect()
    }

    /// Size of a side's panels (width, or height for the bottom).
    pub fn size(&self, side: Side) -> f32 {
        match side {
            Side::Left => self.left_width,
            Side::Right => self.right_width,
            Side::Bottom => self.bottom_height,
        }
    }

    pub fn set_size(&mut self, side: Side, size: f32) {
        let size = size.round();
        match side {
            Side::Left => self.left_width = size,
            Side::Right => self.right_width = size,
            Side::Bottom => self.bottom_height = size,
        }
    }

    /// The upper panel's share when two stack on `side`.
    pub fn split(&self, side: Side) -> f32 {
        match side {
            Side::Left => self.left_split,
            _ => self.right_split,
        }
    }

    pub fn set_split(&mut self, side: Side, split: f32) {
        let split = (split.clamp(0.15, 0.85) * 1000.0).round() / 1000.0;
        match side {
            Side::Left => self.left_split = split,
            _ => self.right_split = split,
        }
    }
}

/// Where a group was drawn in its bar this frame: its area and its buttons, top first.
#[derive(Clone, Debug)]
pub struct DropZone {
    pub group: Group,
    pub rect: egui::Rect,
    pub buttons: Vec<egui::Rect>,
}

/// Where a dragged button dropped at `pos` goes: the nearest group of the bar under the
/// pointer (with some slack sideways), before the first button below the pointer.
pub fn drop_target(zones: &[DropZone], pos: egui::Pos2) -> Option<(Group, usize)> {
    const SLACK: f32 = 14.0;
    let zone = zones
        .iter()
        .filter(|z| z.rect.x_range().expand(SLACK).contains(pos.x))
        .min_by(|a, b| a.rect.distance_to_pos(pos).total_cmp(&b.rect.distance_to_pos(pos)))?;
    Some((zone.group, zone.buttons.iter().filter(|b| b.center().y < pos.y).count()))
}

/// Problems that appeared since the Messages panel was last seen. Problems are
/// identified by a key (a hash of their section and text).
#[derive(Debug, Default)]
pub struct Unseen {
    seen: HashSet<u64>,
    /// Current problems not seen yet.
    pub new: usize,
    /// All current problems.
    pub total: usize,
}

impl Unseen {
    /// Update with the current problems; while `visible` they all count as seen. A
    /// problem that goes away and comes back is new again.
    pub fn update(&mut self, keys: impl IntoIterator<Item = u64>, visible: bool) {
        let current: HashSet<u64> = keys.into_iter().collect();
        if visible {
            self.seen = current.clone();
        } else {
            self.seen.retain(|k| current.contains(k));
        }
        self.new = current.difference(&self.seen).count();
        self.total = current.len();
    }
}

/// Key of a problem for [`Unseen`].
pub fn problem_key(section: &str, text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (section, text).hash(&mut h);
    h.finish()
}

/// Contents of a panel, clipped to its current size. egui grows a resizable panel to fit
/// content that overflows it and keeps that size, so a single frame of overflow (a grid
/// laid out with the previous selection's column widths) would widen the panel for good.
pub fn fixed_size<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let rect = ui.available_rect_before_wrap();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()));
    ui.advance_cursor_after_rect(rect);
    add(&mut child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_panel_per_group() {
        let mut d = Dock::default();
        assert_eq!((d.open(Group::LeftTop), d.open(Group::RightTop)), (Some(Panel::Project), Some(Panel::Properties)));
        d.toggle(Panel::ToolSettings);
        assert_eq!(d.open(Group::RightTop), Some(Panel::ToolSettings), "switches the group's panel");
        assert_eq!(d.open(Group::LeftTop), Some(Panel::Project), "other groups stay");
        d.toggle(Panel::ToolSettings);
        assert_eq!(d.open(Group::RightTop), None, "clicking the open panel hides it");
        d.toggle(Panel::Messages);
        assert!(d.is_open(Panel::Messages));
        assert_eq!(Panel::parse("board-setup"), Some(Panel::BoardSetup));
        assert_eq!(Panel::parse("board_setup"), Some(Panel::BoardSetup));
        assert_eq!(Panel::parse("nope"), None);
        for p in Panel::ALL {
            assert_eq!(Panel::of_action(p.action()), Some(p));
        }
    }

    #[test]
    fn panels_stack_and_move() {
        let mut d = Dock::default();
        d.show(Panel::Nets);
        assert_eq!(d.shown(Side::Right), [(Group::RightTop, Panel::Properties), (Group::RightBottom, Panel::Nets)]);
        // Reorder within a group: Overview before Properties.
        d.move_panel(Panel::Overview, Group::RightTop, 0);
        assert_eq!(d.panels(Group::RightTop)[0], Panel::Overview);
        // Moving down within the group counts positions as they are now.
        d.move_panel(Panel::Overview, Group::RightTop, 2);
        assert_eq!(&d.panels(Group::RightTop)[..3], [Panel::Properties, Panel::Overview, Panel::ToolSettings]);
        // To another side: an open panel stays open there, replacing that group's one.
        d.move_panel(Panel::Properties, Group::LeftBottom, 0);
        assert_eq!(d.group_of(Panel::Properties), Group::LeftBottom);
        assert_eq!(d.shown(Side::Left), [(Group::LeftTop, Panel::Project), (Group::LeftBottom, Panel::Properties)]);
        assert_eq!(d.open(Group::RightTop), None);
        d.move_panel(Panel::Messages, Group::LeftBottom, 9);
        assert_eq!(d.panels(Group::LeftBottom), [Panel::Properties, Panel::Messages]);
    }

    #[test]
    fn dock_saves_loads_and_normalizes() {
        let mut d = Dock::default();
        d.toggle(Panel::Messages);
        d.set_size(Side::Left, 310.4);
        d.move_panel(Panel::Nets, Group::LeftBottom, 0);
        let json = serde_json::to_string(&d).unwrap();
        let back: Dock = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.left_width, 310.0);
        // A file from an older version: missing panels go to their default group,
        // duplicates and open panels of other groups are dropped.
        let mut old: Dock = serde_json::from_str(
            r#"{"groups": {"left_top": ["project", "nets"], "right_top": ["nets", "properties"]},
                "open": {"right_top": "nets"}, "left_split": 7}"#,
        )
        .unwrap();
        old.normalize();
        assert_eq!(old.panels(Group::LeftTop), [Panel::Project, Panel::Nets], "the first place wins");
        assert_eq!(old.panels(Group::RightTop)[..2], [Panel::Properties, Panel::ToolSettings]);
        assert!(old.panels(Group::Bottom).contains(&Panel::Messages));
        let placed: usize = Group::ALL.iter().map(|&g| old.panels(g).len()).sum();
        assert_eq!(placed, Panel::ALL.len(), "every panel once");
        assert_eq!(old.open(Group::RightTop), None);
        assert_eq!(old.left_split, 0.85);
    }

    #[test]
    fn drops_go_before_the_button_below() {
        let button = |y: f32| egui::Rect::from_min_size(egui::pos2(0.0, y), egui::vec2(28.0, 28.0));
        let zones = [
            DropZone {
                group: Group::LeftTop,
                rect: egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(28.0, 64.0)),
                buttons: vec![button(0.0), button(32.0)],
            },
            DropZone {
                group: Group::LeftBottom,
                rect: egui::Rect::from_min_max(egui::pos2(0.0, 80.0), egui::pos2(28.0, 92.0)),
                buttons: vec![],
            },
        ];
        assert_eq!(drop_target(&zones, egui::pos2(14.0, 5.0)), Some((Group::LeftTop, 0)));
        assert_eq!(drop_target(&zones, egui::pos2(14.0, 30.0)), Some((Group::LeftTop, 1)));
        assert_eq!(drop_target(&zones, egui::pos2(14.0, 60.0)), Some((Group::LeftTop, 2)));
        assert_eq!(drop_target(&zones, egui::pos2(14.0, 86.0)), Some((Group::LeftBottom, 0)), "empty group");
        assert_eq!(drop_target(&zones, egui::pos2(14.0, 300.0)), Some((Group::LeftBottom, 0)), "nearest group");
        assert_eq!(drop_target(&zones, egui::pos2(300.0, 30.0)), None, "far from any bar");
    }

    #[test]
    fn overflow_keeps_the_panel_size() {
        // Shows a 300 pt panel whose content wants 400 pt; returns the panel's width.
        let width = |clip: bool| {
            let ctx = egui::Context::default();
            let mut width = 0.0;
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 600.0))),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    let wide = |ui: &mut egui::Ui| {
                        ui.allocate_exact_size(egui::vec2(400.0, 20.0), egui::Sense::hover());
                    };
                    let panel = egui::Panel::right("p").resizable(true).default_size(300.0).size_range(100.0..=700.0);
                    let r = panel.show(ui, |ui| if clip { fixed_size(ui, wide) } else { wide(ui) });
                    width = r.response.rect.width();
                });
                out.textures_delta.clear();
            }
            width
        };
        assert!(width(false) > 400.0, "egui grows the panel to fit overflowing content");
        assert_eq!(width(true), 300.0);
    }

    #[test]
    fn unseen_problems() {
        let mut u = Unseen::default();
        let (a, b, c) = (problem_key("s", "a"), problem_key("s", "b"), problem_key("d", "a"));
        u.update([a, b], false);
        assert_eq!((u.new, u.total), (2, 2));
        u.update([a, b], true);
        assert_eq!(u.new, 0, "seen while the panel is open");
        u.update([a, b, c], false);
        assert_eq!((u.new, u.total), (1, 3), "only the new one counts");
        u.update([b, c], false);
        assert_eq!(u.new, 1);
        u.update([a, b, c], false);
        assert_eq!(u.new, 2, "a went away and came back");
        u.update([], false);
        assert_eq!((u.new, u.total), (0, 0));
    }
}
