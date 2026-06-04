//! Accelerometer → screen rotation (Surface.ROTATION 0/1/2/3), the auto-rotation
//! logic. This is what the framework's `DEVICE_ORIENTATION` (27) fused sensor
//! provides; with the framework off we compute it from the raw accelerometer
//! (a base HAL sensor that survives ART-off). No full attitude fusion is needed
//! for rotation — the gravity vector's angle in the screen plane decides it.
//!
//! Output is the `Surface.ROTATION_*` index the arbiter's `report-orientation`
//! expects (0=natural, 1=90°, 2=180°, 3=270°); the WM module maps it to the
//! content dihedral (`device_rotation_to_orient`).

/// Below this much gravity in the screen (x,y) plane the device is too flat to
/// tell which way is up — hold the last decision. (`g·sin(tilt)`; ~5.5 ≈ 34° tilt.)
const FLAT_GATE: f32 = 5.5;

/// Hysteresis past the 45° quadrant boundary before switching, so a device held
/// near a diagonal doesn't flutter between two rotations.
const HYST_DEG: f32 = 20.0;

/// Consecutive consistent readings required before committing a change (debounce
/// against transient jolts). At ~10 Hz this is ~0.3 s.
const STABLE_COUNT: u8 = 3;

/// The instantaneous quadrant (0/1/2/3) the gravity vector points to in the screen
/// plane, or `None` if too flat. `angle` is measured from +y (portrait-up): 0→+y,
/// 90→+x, 180→-y, 270→-x. The quadrant→rotation map is the one device-specific
/// handedness knob (mirrors the caveat on the host's `device_rotation_to_orient`).
fn instantaneous(x: f32, y: f32) -> Option<u32> {
    let planar = (x * x + y * y).sqrt();
    if planar < FLAT_GATE {
        return None;
    }
    // Direction of "up" (opposes gravity; the accelerometer reads +g along it).
    let deg = x.atan2(y).to_degrees().rem_euclid(360.0); // 0 at +y, CCW toward +x
    let q = (((deg / 90.0).round() as i32) & 3) as u32;
    Some(rotation_for_quadrant(q))
}

/// Map the up-vector quadrant to a `Surface.ROTATION_*` index. Device-tunable: if
/// the screen rotates the wrong way on a given panel, adjust here (parallels the
/// host `device_rotation_to_orient` handedness caveat).
fn rotation_for_quadrant(q: u32) -> u32 {
    match q {
        0 => 0, // up = +y  → natural portrait
        1 => 3, // up = +x  → ROTATION_270
        2 => 2, // up = -y  → ROTATION_180
        _ => 1, // up = -x  → ROTATION_90
    }
}

/// True if `deg` (angle from +y, degrees in [0,360)) is within `HYST_DEG` of a
/// quadrant boundary (45/135/225/315) — i.e. ambiguous; don't switch there.
fn near_boundary(x: f32, y: f32) -> bool {
    let deg = x.atan2(y).to_degrees().rem_euclid(360.0);
    let off = (deg + 45.0).rem_euclid(90.0); // distance into the current 90° cell
    off < HYST_DEG || off > (90.0 - HYST_DEG)
}

/// Stateful rotation tracker with hysteresis + debounce. Feed accelerometer
/// samples; it returns `Some(rot)` exactly when the committed rotation changes.
#[derive(Debug, Default)]
pub struct OrientationTracker {
    current: Option<u32>,
    candidate: Option<u32>,
    streak: u8,
}

impl OrientationTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// The committed rotation so far (None until the first decision).
    pub fn current(&self) -> Option<u32> {
        self.current
    }

    /// Directly record the current rotation (used when a HAL-fused DEVICE_ORIENTATION
    /// sensor already debounced it, so no gravity-vector tracking is needed).
    pub fn set(&mut self, rot: u32) {
        self.current = Some(rot);
        self.candidate = None;
        self.streak = 0;
    }

    /// Feed one accelerometer sample (m/s², device frame). Returns `Some(rot)`
    /// only on a committed change (so the caller pushes `report-orientation` only
    /// when it actually changes).
    pub fn update(&mut self, x: f32, y: f32, _z: f32) -> Option<u32> {
        let reading = instantaneous(x, y);
        let Some(rot) = reading else {
            // Too flat — keep the current decision, reset any pending candidate.
            self.candidate = None;
            self.streak = 0;
            return None;
        };
        // Near a 45° boundary while it would mean a *change*: stay put (hysteresis).
        if self.current == Some(rot) {
            self.candidate = None;
            self.streak = 0;
            return None;
        }
        if near_boundary(x, y) {
            return None;
        }
        // Debounce: require STABLE_COUNT consecutive agreeing samples.
        if self.candidate == Some(rot) {
            self.streak = self.streak.saturating_add(1);
        } else {
            self.candidate = Some(rot);
            self.streak = 1;
        }
        if self.streak >= STABLE_COUNT {
            self.current = Some(rot);
            self.candidate = None;
            self.streak = 0;
            return Some(rot);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: f32 = 9.81;

    #[test]
    fn instantaneous_quadrants() {
        assert_eq!(instantaneous(0.0, G, ), Some(0)); // up=+y portrait
        assert_eq!(instantaneous(G, 0.0), Some(3)); // up=+x
        assert_eq!(instantaneous(0.0, -G), Some(2)); // up=-y
        assert_eq!(instantaneous(-G, 0.0), Some(1)); // up=-x
    }

    #[test]
    fn flat_is_none() {
        // Device lying flat: gravity almost all on z, tiny planar component.
        assert_eq!(instantaneous(0.3, 0.3), None);
    }

    #[test]
    fn tracker_debounces_then_commits() {
        let mut t = OrientationTracker::new();
        // First decision (portrait) needs STABLE_COUNT samples.
        assert_eq!(t.update(0.0, G, 0.0), None);
        assert_eq!(t.update(0.0, G, 0.0), None);
        assert_eq!(t.update(0.0, G, 0.0), Some(0));
        assert_eq!(t.current(), Some(0));
        // Staying portrait → no event.
        assert_eq!(t.update(0.2, G, 0.0), None);
        // Rotate to landscape (+x up = rot 3): debounced.
        assert_eq!(t.update(G, 0.0, 0.0), None);
        assert_eq!(t.update(G, 0.0, 0.0), None);
        assert_eq!(t.update(G, 0.0, 0.0), Some(3));
    }

    #[test]
    fn near_boundary_holds() {
        let mut t = OrientationTracker::new();
        // commit portrait
        for _ in 0..STABLE_COUNT {
            t.update(0.0, G, 0.0);
        }
        // exactly diagonal (45°): ambiguous → never commits a change
        for _ in 0..10 {
            assert_eq!(t.update(G, G, 0.0), None);
        }
        assert_eq!(t.current(), Some(0));
    }

    #[test]
    fn transient_jolt_does_not_switch() {
        let mut t = OrientationTracker::new();
        for _ in 0..STABLE_COUNT {
            t.update(0.0, G, 0.0);
        }
        // one stray landscape sample then back to portrait → no commit
        assert_eq!(t.update(G, 0.0, 0.0), None);
        assert_eq!(t.update(0.0, G, 0.0), None);
        assert_eq!(t.current(), Some(0));
    }
}
