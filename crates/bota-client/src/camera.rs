//! The view onto the map.

/// Where the player looks and how closely.
///
/// World coordinates grow up and to the right; screen coordinates grow down,
/// so the vertical axis flips in the transform.
pub struct Camera {
    /// World X at the center of the screen.
    pub x: f32,
    /// World Y at the center of the screen.
    pub y: f32,
    /// Screen pixels per world unit.
    pub zoom: f32,
}

/// The world spans this many units per axis; the server's `MAP_SIZE`.
pub const MAP_SIZE: f32 = 18432.0;

/// Closest allowed zoom, pixels per world unit.
pub const ZOOM_MAX: f32 = 1.5;
/// Farthest allowed zoom.
pub const ZOOM_MIN: f32 = 0.06;

impl Camera {
    /// A camera over a point at the default zoom.
    pub fn over(x: f32, y: f32) -> Camera {
        Camera { x, y, zoom: 0.45 }
    }

    /// Screen position of a world point, given the screen size.
    pub fn world_to_screen(&self, wx: f32, wy: f32, sw: f32, sh: f32) -> (f32, f32) {
        (
            (wx - self.x) * self.zoom + sw / 2.0,
            sh / 2.0 - (wy - self.y) * self.zoom,
        )
    }

    /// World position under a screen point, given the screen size.
    pub fn screen_to_world(&self, sx: f32, sy: f32, sw: f32, sh: f32) -> (f32, f32) {
        (
            (sx - sw / 2.0) / self.zoom + self.x,
            (sh / 2.0 - sy) / self.zoom + self.y,
        )
    }

    /// Glides towards a point; a second of gliding closes most of the gap.
    pub fn follow(&mut self, wx: f32, wy: f32, dt: f32) {
        let k = (dt * 8.0).min(1.0);
        self.x += (wx - self.x) * k;
        self.y += (wy - self.y) * k;
    }

    /// Zooms by a wheel step, staying within the limits.
    pub fn zoom_by(&mut self, steps: f32) {
        self.zoom = (self.zoom * (1.15f32).powf(steps)).clamp(ZOOM_MIN, ZOOM_MAX);
    }

    /// Pans by a screen-space distance.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        self.x += dx / self.zoom;
        self.y -= dy / self.zoom;
    }
}
