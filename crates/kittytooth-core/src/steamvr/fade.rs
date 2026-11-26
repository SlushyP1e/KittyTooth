//! "Fade when not looking": the wrist overlay fades out unless you look at it, like a watch.
//!
//! Pure math, no SteamVR calls: poses are row-major 3x4 matrices (`HmdMatrix34_t`), where the
//! last column is the position and a device looks down its -Z axis.

pub type Matrix34 = [[f32; 4]; 3];

/// Affine product `a * b` (apply `b`, then `a`).
pub fn mul(a: &Matrix34, b: &Matrix34) -> Matrix34 {
    let mut out = [[0.0f32; 4]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[r][k] * b[k][c]).sum::<f32>() + if c == 3 { a[r][3] } else { 0.0 };
        }
    }
    out
}

fn pos(m: &Matrix34) -> [f32; 3] {
    [m[0][3], m[1][3], m[2][3]]
}

fn col(m: &Matrix34, c: usize) -> [f32; 3] {
    [m[0][c], m[1][c], m[2][c]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Angle between two vectors in degrees (0 for a zero-length vector).
fn angle_deg(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dot: f32 = (0..3).map(|i| a[i] * b[i]).sum();
    let la = (0..3).map(|i| a[i] * a[i]).sum::<f32>().sqrt();
    let lb = (0..3).map(|i| b[i] * b[i]).sum::<f32>().sqrt();
    if la < 1e-6 || lb < 1e-6 {
        return 0.0;
    }
    (dot / (la * lb)).clamp(-1.0, 1.0).acos().to_degrees()
}

/// How far (degrees) the overlay is from the centre of your view.
pub fn gaze_angle(hmd: &Matrix34, overlay: &Matrix34) -> f32 {
    let forward = col(hmd, 2).map(|v| -v);
    angle_deg(forward, sub(pos(overlay), pos(hmd)))
}

/// How far (degrees) the overlay's front is turned away from your eyes (0 = facing you).
pub fn facing_angle(hmd: &Matrix34, overlay: &Matrix34) -> f32 {
    angle_deg(col(overlay, 2), sub(pos(hmd), pos(overlay)))
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Gaze angles (fully visible within, gone beyond) for a sensitivity setting.
pub fn fade_angles(sensitivity: &str) -> (f32, f32) {
    match sensitivity {
        "narrow" => (15.0, 30.0),
        "wide" => (30.0, 50.0),
        _ => (20.0, 38.0),
    }
}

/// Target visibility 0..1: full within `inner` degrees of your view, none beyond `outer`, smooth in
/// between. Also hidden when the card is turned edge-on or away from you.
pub fn fade_target(gaze_deg: f32, facing_deg: f32, (inner, outer): (f32, f32)) -> f32 {
    let gaze = 1.0 - smoothstep(inner, outer, gaze_deg);
    let facing = 1.0 - smoothstep(75.0, 100.0, facing_deg);
    gaze * facing
}

/// Moves `current` towards `target` so a full fade takes `secs` (no popping).
pub fn ease_toward(current: f32, target: f32, dt_secs: f32, secs: f32) -> f32 {
    let step = if secs <= 0.0 { 1.0 } else { dt_secs / secs };
    if (target - current).abs() <= step {
        target
    } else {
        current + step.copysign(target - current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const I: Matrix34 = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

    fn at(x: f32, y: f32, z: f32) -> Matrix34 {
        [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, z]]
    }

    fn rot_y(deg: f32) -> Matrix34 {
        let (s, c) = deg.to_radians().sin_cos();
        [[c, 0.0, s, 0.0], [0.0, 1.0, 0.0, 0.0], [-s, 0.0, c, 0.0]]
    }

    #[test]
    fn mul_composes_translation_and_rotation() {
        let m = mul(&at(1.0, 2.0, 3.0), &at(0.5, 0.0, -1.0));
        assert_eq!(pos(&m), [1.5, 2.0, 2.0]);
        // Rotating 90° about Y turns +Z offset into +X.
        let m = mul(&rot_y(90.0), &at(0.0, 0.0, 1.0));
        assert!((m[0][3] - 1.0).abs() < 1e-5 && m[2][3].abs() < 1e-5);
        assert_eq!(mul(&I, &I), I);
    }

    #[test]
    fn gaze_and_facing_angles() {
        // HMD at origin looking down -Z; overlay 1 m ahead, facing back at us (+Z).
        let hmd = I;
        assert!(gaze_angle(&hmd, &at(0.0, 0.0, -1.0)) < 0.01);
        assert!(facing_angle(&hmd, &at(0.0, 0.0, -1.0)) < 0.01);
        // 1 m ahead and 1 m to the side: 45° off.
        assert!((gaze_angle(&hmd, &at(1.0, 0.0, -1.0)) - 45.0).abs() < 0.01);
        // Behind us.
        assert!(gaze_angle(&hmd, &at(0.0, 0.0, 1.0)) > 179.0);
        // Turned 90° away is edge-on.
        let edge = mul(&at(0.0, 0.0, -1.0), &rot_y(90.0));
        assert!((facing_angle(&hmd, &edge) - 90.0).abs() < 0.01);
        // Turning the head 45° to look at the side overlay brings it to the centre.
        assert!(gaze_angle(&rot_y(-45.0), &at(1.0, 0.0, -1.0)) < 0.01);
    }

    #[test]
    fn target_ramps_smoothly() {
        let a = fade_angles("normal");
        assert_eq!(fade_target(0.0, 0.0, a), 1.0);
        assert_eq!(fade_target(20.0, 0.0, a), 1.0);
        assert_eq!(fade_target(38.0, 0.0, a), 0.0);
        assert_eq!(fade_target(90.0, 0.0, a), 0.0);
        let mid = fade_target(29.0, 0.0, a);
        assert!(mid > 0.3 && mid < 0.7, "{mid}");
        // Monotonic as you look away.
        let mut last = 1.0;
        for d in 0..60 {
            let v = fade_target(d as f32, 0.0, a);
            assert!(v <= last + 1e-6);
            last = v;
        }
        // Back-facing card is hidden even when centred.
        assert_eq!(fade_target(0.0, 120.0, a), 0.0);
        assert!(fade_angles("narrow").1 < fade_angles("wide").0 + 1.0);
    }

    #[test]
    fn easing_takes_the_set_time() {
        let mut a = 0.0;
        let mut steps = 0;
        while a < 1.0 {
            a = ease_toward(a, 1.0, 0.025, 0.2);
            steps += 1;
            assert!(steps < 100);
        }
        assert_eq!(steps, 8, "0.2 s at 40 Hz");
        assert_eq!(ease_toward(0.5, 0.5, 0.025, 0.2), 0.5);
        assert!((ease_toward(1.0, 0.0, 0.025, 0.2) - 0.875).abs() < 1e-6);
        assert_eq!(ease_toward(0.3, 0.9, 0.1, 0.0), 0.9);
    }
}
