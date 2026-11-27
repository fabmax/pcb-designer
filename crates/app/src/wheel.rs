//! Which device a browser wheel event came from. Browsers don't say (eframe passes
//! `MouseWheelSource::Unknown`), and Chrome and Safari report mouse wheels in pixels
//! like trackpads, so the canvas would pan on a mouse wheel instead of zooming.

use egui::MouseWheelSource;

/// A wheel event's fields (`WheelEvent`): `deltaMode`, `deltaX`, `deltaY` and the legacy
/// `wheelDeltaY` (Chrome, Safari, Firefox; `None` where missing).
pub fn classify(delta_mode: u32, delta_x: f64, delta_y: f64, wheel_delta_y: Option<f64>) -> MouseWheelSource {
    const PIXEL: u32 = 0;
    if delta_mode != PIXEL {
        return MouseWheelSource::Wheel; // lines or pages: a mouse wheel (Firefox)
    }
    if delta_x != 0.0 {
        return MouseWheelSource::Trackpad; // mice scroll vertically
    }
    match wheel_delta_y {
        // A trackpad's legacy delta is exactly -3× its pixels (rounded); a mouse wheel's
        // counts notches (multiples of 120) whatever the pixels.
        Some(w) if w != 0.0 => {
            if (w + 3.0 * delta_y).abs() <= 1.0 {
                MouseWheelSource::Trackpad
            } else {
                MouseWheelSource::Wheel
            }
        }
        _ => MouseWheelSource::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notches_of_a_mouse_wheel_are_a_wheel() {
        // Chrome on Windows: 100 px a notch; on macOS accelerated pixels.
        assert_eq!(classify(0, 0.0, 100.0, Some(-120.0)), MouseWheelSource::Wheel);
        assert_eq!(classify(0, 0.0, -4.0, Some(120.0)), MouseWheelSource::Wheel);
        assert_eq!(classify(0, 0.0, 12.5, Some(-240.0)), MouseWheelSource::Wheel);
        // Firefox: lines.
        assert_eq!(classify(1, 0.0, 3.0, Some(-120.0)), MouseWheelSource::Wheel);
    }

    #[test]
    fn trackpad_scrolling_is_a_trackpad() {
        assert_eq!(classify(0, 0.0, 2.0, Some(-6.0)), MouseWheelSource::Trackpad);
        assert_eq!(classify(0, 0.0, 1.3333, Some(-4.0)), MouseWheelSource::Trackpad, "rounded");
        assert_eq!(classify(0, 3.0, 0.0, None), MouseWheelSource::Trackpad, "sideways");
    }

    #[test]
    fn without_the_legacy_delta_it_is_unknown() {
        assert_eq!(classify(0, 0.0, 5.0, None), MouseWheelSource::Unknown);
    }
}
