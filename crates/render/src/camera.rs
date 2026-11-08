//! 2D camera: maps world coordinates (millimeters, Y up) to screen pixels (Y down).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// World position at the center of the viewport.
    pub center: [f64; 2],
    /// Pixels per world unit.
    pub zoom: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            zoom: 4.0,
        }
    }
}

impl Camera {
    pub const MIN_ZOOM: f64 = 0.01;
    pub const MAX_ZOOM: f64 = 50_000.0;

    /// Screen position (pixels, relative to the viewport's top left corner) → world.
    pub fn screen_to_world(&self, screen: [f64; 2], viewport: [f64; 2]) -> [f64; 2] {
        [
            self.center[0] + (screen[0] - viewport[0] * 0.5) / self.zoom,
            self.center[1] - (screen[1] - viewport[1] * 0.5) / self.zoom,
        ]
    }

    /// World → screen position (pixels, relative to the viewport's top left corner).
    pub fn world_to_screen(&self, world: [f64; 2], viewport: [f64; 2]) -> [f64; 2] {
        [
            (world[0] - self.center[0]) * self.zoom + viewport[0] * 0.5,
            -(world[1] - self.center[1]) * self.zoom + viewport[1] * 0.5,
        ]
    }

    /// Pan by a screen space delta in pixels.
    pub fn pan_pixels(&mut self, delta: [f64; 2]) {
        self.center[0] -= delta[0] / self.zoom;
        self.center[1] += delta[1] / self.zoom;
    }

    /// Zoom by `factor`, keeping the world point under `anchor` (screen pixels) fixed.
    pub fn zoom_at(&mut self, factor: f64, anchor: [f64; 2], viewport: [f64; 2]) {
        let before = self.screen_to_world(anchor, viewport);
        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let after = self.screen_to_world(anchor, viewport);
        self.center[0] += before[0] - after[0];
        self.center[1] += before[1] - after[1];
    }

    /// Fit the world rectangle `[min_x, min_y, max_x, max_y]` into the viewport with a margin.
    pub fn fit(&mut self, bounds: [f64; 4], viewport: [f64; 2], margin_px: f64) {
        let w = (bounds[2] - bounds[0]).max(1e-6);
        let h = (bounds[3] - bounds[1]).max(1e-6);
        let zx = (viewport[0] - 2.0 * margin_px).max(1.0) / w;
        let zy = (viewport[1] - 2.0 * margin_px).max(1.0) / h;
        self.zoom = zx.min(zy).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        self.center = [(bounds[0] + bounds[2]) * 0.5, (bounds[1] + bounds[3]) * 0.5];
    }

    /// Visible world rectangle `[min_x, min_y, max_x, max_y]`.
    pub fn visible_world(&self, viewport: [f64; 2]) -> [f64; 4] {
        let hw = viewport[0] * 0.5 / self.zoom;
        let hh = viewport[1] * 0.5 / self.zoom;
        [
            self.center[0] - hw,
            self.center[1] - hh,
            self.center[0] + hw,
            self.center[1] + hh,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_world_roundtrip() {
        let cam = Camera {
            center: [10.0, -5.0],
            zoom: 3.0,
        };
        let vp = [800.0, 600.0];
        let w = cam.screen_to_world([123.0, 456.0], vp);
        let s = cam.world_to_screen(w, vp);
        assert!((s[0] - 123.0).abs() < 1e-9 && (s[1] - 456.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let mut cam = Camera {
            center: [0.0, 0.0],
            zoom: 2.0,
        };
        let vp = [800.0, 600.0];
        let anchor = [100.0, 50.0];
        let before = cam.screen_to_world(anchor, vp);
        cam.zoom_at(1.7, anchor, vp);
        let after = cam.screen_to_world(anchor, vp);
        assert!((before[0] - after[0]).abs() < 1e-9 && (before[1] - after[1]).abs() < 1e-9);
    }
}
