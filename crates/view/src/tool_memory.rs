//! The drawing tools' style choices, remembered across sessions: corner styles, widths,
//! sizes, layers and the like. Transient fields (rotation, the next label or pin name,
//! the active routing layer) are `#[serde(skip)]` in the settings structs and come back
//! as their defaults.
//!
//! Each tool's settings are kept as a JSON value, so that a field renamed or removed in a
//! later version only loses itself, never the rest of the settings file.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// The settings to remember (their remembered fields only).
pub fn remember<T: Serialize>(settings: &T) -> Value {
    serde_json::to_value(settings).unwrap_or(Value::Null)
}

/// Remembered settings, or the defaults for what is missing or unreadable.
pub fn recall<T: DeserializeOwned + Default>(value: Option<&Value>) -> T {
    match value {
        Some(v) => T::deserialize(v).unwrap_or_default(),
        None => T::default(),
    }
}

#[cfg(test)]
mod tests {
    use model::*;

    use super::*;
    use crate::bend::Corners;
    use crate::pcb_tools::PcbSettings;
    use crate::sch_tools::SchSettings;
    use crate::tools::Settings;

    #[test]
    fn style_is_kept_transient_fields_are_not() {
        let mut pcb = PcbSettings {
            corners: Corners::Arc90,
            width: Some(mm(0.4)),
            layer: LayerId::BOTTOM_COPPER,
            zone_net: "GND".into(),
            ..Default::default()
        };
        pcb.rotation = Angle::DEG90;
        let back: PcbSettings = recall(Some(&remember(&pcb)));
        assert_eq!((back.corners, back.width), (Corners::Arc90, Some(mm(0.4))));
        assert_eq!(back.layer, LayerId::TOP_COPPER, "the routing layer starts on top");
        assert!(back.zone_net.is_empty() && back.rotation == Angle::ZERO);

        let sch = SchSettings { label: "SDA".into(), text_size: mm(2.0), ..Default::default() };
        let back: SchSettings = recall(Some(&remember(&sch)));
        assert_eq!((back.label.as_str(), back.text_size), ("NET1", mm(2.0)));

        let lib = Settings { name: "7".into(), pad_size: Size::mm(2.0, 1.0), ..Default::default() };
        let back: Settings = recall(Some(&remember(&lib)));
        assert_eq!((back.name.as_str(), back.pad_size), ("1", Size::mm(2.0, 1.0)));
    }

    #[test]
    fn unreadable_values_fall_back() {
        // A field of the wrong type loses this tool's settings, nothing else.
        let v = serde_json::json!({ "corners": 7, "text_size": 1 });
        let s: SchSettings = recall(Some(&v));
        assert_eq!(s.text_size, SchSettings::default().text_size);
        // Unknown fields are ignored.
        let v = serde_json::json!({ "gone": true, "text_size": 1 });
        assert_eq!(recall::<SchSettings>(Some(&v)).text_size, 1);
        assert_eq!(recall::<SchSettings>(None).corners, SchSettings::default().corners);
    }
}
