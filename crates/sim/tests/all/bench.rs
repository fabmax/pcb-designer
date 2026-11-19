//! Test circuits built with commands: parts from the Built-in library, their pins joined
//! by labels at the pin ends (global labels, `GND` a ground power port).

use editor_core::connectivity::Connectivity;
use editor_core::query::pin_positions;
use editor_core::{Command, Editor};
use model::builtin::builtin_library;
use model::*;

pub struct Bench {
    pub ed: Editor,
    x: f64,
}

/// A Built-in part by name.
pub fn part(name: &str) -> Id {
    builtin_library().parts.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("no part {name}")).id
}

impl Bench {
    pub fn new() -> Bench {
        let lib = builtin_library();
        let ids: Vec<Id> = lib.parts.iter().map(|p| p.id).collect();
        let mut ed = Editor::new(Project::new("bench"));
        ed.execute(Command::import_parts(&lib, &ids).unwrap()).unwrap();
        Bench { ed, x: 0.0 }
    }

    pub fn project(&self) -> &Project {
        self.ed.project()
    }

    pub fn conn(&self) -> Connectivity {
        Connectivity::build(self.project())
    }

    /// Places `part` (by name) with `value` and labels its first unit's pins, in the
    /// symbol's order, with `nets` ("" leaves a pin open). Returns the component and symbol.
    pub fn place(&mut self, part_name: &str, value: &str, nets: &[&str]) -> (Id, Id) {
        self.x += 40.0;
        let changes = self
            .ed
            .execute(Command::PlacePart {
                part: part(part_name),
                variant: None,
                sheet: None,
                pos: Point::mm(self.x, 100.0),
                rotation: Angle::ZERO,
                mirror: false,
                refdes: None,
                value: (!value.is_empty()).then(|| value.to_string()),
            })
            .unwrap();
        let symbol = changes.ids().find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None }).unwrap();
        self.label_pins(symbol, nets);
        (self.project().schematic.symbols.get(symbol).unwrap().component, symbol)
    }

    /// Places another unit of `component` and labels its pins.
    pub fn place_unit(&mut self, component: Id, unit: u32, nets: &[&str]) -> Id {
        self.x += 40.0;
        let changes = self
            .ed
            .execute(Command::PlaceUnit {
                component,
                unit,
                sheet: None,
                pos: Point::mm(self.x, 100.0),
                rotation: Angle::ZERO,
                mirror: false,
            })
            .unwrap();
        let symbol = changes.ids().find_map(|id| if let ObjectId::Symbol(s) = id { Some(s) } else { None }).unwrap();
        self.label_pins(symbol, nets);
        symbol
    }

    fn label_pins(&mut self, symbol: Id, nets: &[&str]) {
        let pins = pin_positions(self.project(), symbol);
        assert!(nets.len() <= pins.len(), "{} nets for {} pins", nets.len(), pins.len());
        for ((_, _, pos), net) in pins.iter().zip(nets) {
            if net.is_empty() {
                continue;
            }
            let kind = if *net == "GND" { LabelKind::Power } else { LabelKind::Global };
            self.ed
                .execute(Command::AddLabel {
                    sheet: None,
                    pos: *pos,
                    name: net.to_string(),
                    kind,
                    shape: LabelShape::Passive,
                    style: None,
                    rotation: Angle::ZERO,
                })
                .unwrap();
        }
    }

    /// The pin of a placed symbol by name.
    pub fn pin(&self, symbol: Id, name: &str) -> Id {
        pin_positions(self.project(), symbol).into_iter().find(|p| p.1 == name).unwrap().0
    }

    pub fn component(&self, id: Id) -> Component {
        self.project().schematic.components.get(id).unwrap().clone()
    }

    pub fn update_component(&mut self, id: Id, f: impl FnOnce(&mut Component)) {
        let mut c = self.component(id);
        f(&mut c);
        self.ed.execute(Command::Update { objects: vec![Object::Component(c)] }).unwrap();
    }

    /// A source part with `waveform` (and an AC magnitude).
    pub fn source(&mut self, part_name: &str, waveform: Waveform, ac: f64, nets: &[&str]) -> Id {
        let (c, _) = self.place(part_name, "", nets);
        self.update_component(c, |c| c.sim.source = Some(Source { waveform, ac, ac_phase: 0.0 }));
        c
    }

    pub fn dc(&mut self, volts: f64, plus: &str, minus: &str) -> Id {
        self.source("VSource", Waveform::Dc { value: volts }, 0.0, &[plus, minus])
    }

    /// Adds an analysis to the setup and returns it.
    pub fn analysis(&mut self, kind: AnalysisKind) -> Analysis {
        let mut sim = self.project().schematic.simulation.clone();
        let a = Analysis::new(format!("{} {}", kind.label(), sim.analyses.len() + 1), kind);
        sim.analyses.push(a.clone());
        self.ed.execute(Command::SetSimulation { simulation: sim }).unwrap();
        a
    }
}
