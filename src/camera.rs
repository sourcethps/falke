use std::ops::{Add, Mul, Sub};

use nalgebra_glm as glm;

use crate::math::{Quat, Vec3, lerp, lerp_arr};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    pub position: Vec3,
    pub orientation: Quat,
    pub fov: f32,
}

impl CameraPose {
    /// Blend two poses: lerp the position and fov, slerp the rotation.
    pub fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        Self {
            position: Vec3::from_array(lerp_arr(a.position.to_array(), b.position.to_array(), t)),
            orientation: glm::quat_slerp(&a.orientation.into(), &b.orientation.into(), t).into(),
            fov: lerp(a.fov, b.fov, t),
        }
    }
}

/// A camera pose pinned to a point in time, plus how tightly the spline through
/// it should turn.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub time: f32,
    pub pose: CameraPose,
    /// 0 = a loose, rounded curve through this point, 1 = a hard corner.
    pub tension: f32,
    /// Set when the key was placed while orbiting. Runs of orbit keys then
    /// interpolate yaw, pitch and distance, so the camera sweeps round the
    /// pivot instead of cutting across the circle. `pose` keeps the snapshot.
    pub orbit: Option<OrbitCamera>,
}

impl Keyframe {
    /// Where this key puts the camera, given the skater's current position.
    pub fn resolve(&self, skater: Option<Vec3>) -> CameraPose {
        match self.orbit {
            Some(orbit) => orbit.pose_around(orbit.pivot(skater), self.pose.fov),
            None => self.pose,
        }
    }
}
/// An ordered, sorted sequence of camera poses that can be sampled at any time.
#[derive(Default)]
pub struct CameraPath {
    frames: Vec<Keyframe>,
}

impl CameraPath {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop every keyframe.
    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// Add a keyframe and re-sort to maintain the time invariant.
    pub fn push(&mut self, kf: Keyframe) {
        self.frames.push(kf);
        self.sort();
    }

    /// Remove the keyframe at `index` and return it.
    pub fn remove(&mut self, index: usize) -> Keyframe {
        self.frames.remove(index)
    }

    pub fn frames(&self) -> &[Keyframe] {
        &self.frames
    }

    pub fn get(&self, index: usize) -> Option<&Keyframe> {
        self.frames.get(index)
    }

    /// Mutable access to a single frame — use for live editing, then call `sort()`.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Keyframe> {
        self.frames.get_mut(index)
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Re-sort frames by time. Required after mutating a frame's time via `get_mut`.
    pub fn sort(&mut self) {
        self.frames.sort_by(|a, b| {
            a.time
                .partial_cmp(&b.time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Sample the path at `time`. `skater` is the pivot for orbit keys that
    /// follow the skater. Returns `None` when past the last keyframe or when
    /// fewer than two keyframes are present.
    pub fn sample(&self, time: f32, skater: Option<Vec3>) -> Option<CameraPose> {
        sample_frames(&self.frames, time, skater)
    }
}

fn sample_frames(frames: &[Keyframe], time: f32, skater: Option<Vec3>) -> Option<CameraPose> {
    let len = frames.len();
    if len < 2 {
        return None;
    }

    if time <= frames[0].time {
        return Some(frames[0].resolve(skater));
    }
    if time == frames[len - 1].time {
        return Some(frames[len - 1].resolve(skater));
    }
    if time > frames[len - 1].time {
        return None;
    }

    let i2 = frames.partition_point(|f| f.time < time).max(1);
    let i1 = i2 - 1;

    let f0 = if i1 > 0 { &frames[i1 - 1] } else { &frames[i1] };
    let f1 = &frames[i1];
    let f2 = &frames[i2];
    let f3 = if i2 + 1 < len {
        &frames[i2 + 1]
    } else {
        &frames[i2]
    };

    let t = (time - f1.time) / (f2.time - f1.time);
    let keys = [f0, f1, f2, f3];

    if let Some(pose) = sample_orbit(keys, t, skater) {
        return Some(pose);
    }

    // World space. Orbit keys still take part, at wherever they currently put
    // the camera, so a run can hand over to or from free keys.
    let poses = keys.map(|k| k.resolve(skater));
    let times = keys.map(|k| k.time);

    let positions = poses.map(|p| glm::Vec3::from(p.position));
    let position = hermite(positions, times, t, f1.tension).into();
    let orientation = if len > 3 {
        interpolate_rotation(poses, times, t)
    } else {
        glm::quat_slerp(
            &poses[1].orientation.into(),
            &poses[2].orientation.into(),
            t,
        )
        .into()
    };
    let fov = lerp(f1.pose.fov, f2.pose.fov, t);

    Some(CameraPose {
        position,
        orientation,
        fov,
    })
}

/// Interpolate in orbit space when both ends of the segment are orbit keys
/// with the same pivot. Yaw is not wrapped, so a key at 0° and one at 720°
/// spin twice round the target.
fn sample_orbit(keys: [&Keyframe; 4], t: f32, skater: Option<Vec3>) -> Option<CameraPose> {
    let (k1, k2) = (keys[1].orbit?, keys[2].orbit?);

    // A neighbour outside the orbit run acts like the end of the path.
    let neighbour = |k: &Keyframe, inner: OrbitCamera, inner_time: f32| match k.orbit {
        Some(o) => (o, k.time),
        None => (inner, inner_time),
    };
    let (o0, t0) = neighbour(keys[0], k1, keys[1].time);
    let (o3, t3) = neighbour(keys[3], k2, keys[2].time);
    let orbits = [o0, k1, k2, o3];
    let times = [t0, keys[1].time, keys[2].time, t3];
    let tension = keys[1].tension;

    let spline =
        |field: fn(&OrbitCamera) -> f32| hermite(orbits.map(|o| field(&o)), times, t, tension);
    let targets = orbits.map(|o| glm::Vec3::from(o.target));
    let orbit = OrbitCamera {
        target: hermite(targets, times, t, tension).into(),
        yaw: spline(|o| o.yaw),
        pitch: spline(|o| o.pitch).clamp(-MAX_PITCH, MAX_PITCH),
        distance: spline(|o| o.distance).max(OrbitCamera::MIN_DISTANCE),
    };
    let fov = lerp(keys[1].pose.fov, keys[2].pose.fov, t);
    Some(orbit.pose_around(orbit.pivot(skater), fov))
}

/// Cubic Hermite through `p[1]..p[2]`, with `p[0]` and `p[3]` shaping the
/// tangents.
fn hermite<T>(p: [T; 4], times: [f32; 4], t: f32, tension: f32) -> T
where
    T: Copy + Add<Output = T> + Sub<Output = T> + Mul<f32, Output = T>,
{
    let scale = (times[2] - times[1]) * (1.0 - tension);
    let m1 = (p[2] - p[0]) * (scale / (times[2] - times[0]).max(f32::EPSILON));
    let m2 = (p[3] - p[1]) * (scale / (times[3] - times[1]).max(f32::EPSILON));

    let t2 = t * t;
    let t3 = t2 * t;

    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;

    p[1] * h00 + m1 * h10 + p[2] * h01 + m2 * h11
}

fn squad_tangent(q_prev: glm::Quat, q: glm::Quat, q_next: glm::Quat) -> glm::Quat {
    let inv_q = glm::quat_inverse(&q);
    let log1 = glm::quat_log(&(inv_q * q_prev));
    let log2 = glm::quat_log(&(inv_q * q_next));
    q * glm::quat_exp(&((-0.25) * (log1 + log2)))
}

fn squad(q1: glm::Quat, q2: glm::Quat, s1: glm::Quat, s2: glm::Quat, t: f32) -> glm::Quat {
    let slerp_1 = glm::quat_slerp(&q1, &q2, t);
    let slerp_2 = glm::quat_slerp(&s1, &s2, t);
    glm::quat_slerp(&slerp_1, &slerp_2, 2.0 * t * (1.0 - t))
}

fn interpolate_rotation(poses: [CameraPose; 4], times: [f32; 4], t: f32) -> Quat {
    let q1: glm::Quat = poses[1].orientation.into();
    let q2 = ensure_shortest_path(q1, poses[2].orientation.into());

    let use_slerp = times[0] == times[1];
    if use_slerp {
        return glm::quat_slerp(&q1, &q2, t).into();
    }

    let q0 = ensure_shortest_path(q1, poses[0].orientation.into());
    let q3 = ensure_shortest_path(q2, poses[3].orientation.into());

    let s1 = squad_tangent(q0, q1, q2);
    let s2 = squad_tangent(q1, q2, q3);

    squad(q1, q2, s1, s2, t).into()
}

/// `q` and `-q` are the same rotation; pick the sign that blends the short way
/// round, or the camera spins through 360°.
fn ensure_shortest_path(q1: glm::Quat, q2: glm::Quat) -> glm::Quat {
    if glm::quat_dot(&q1, &q2) < 0.0 {
        -q2
    } else {
        q2
    }
}

// Look-at

/// The engine is Y-up.
fn world_up() -> glm::Vec3 {
    glm::vec3(0.0, 1.0, 0.0)
}

/// if orbit and tracking cameras face away from their target.
const VIEW_AXIS_SIGN: f32 = -1.0;

/// Steepest the orbit may look up or down. Straight along the up axis the
/// look-at basis degenerates.
const MAX_PITCH: f32 = 1.55;

/// The direction a camera with this orientation looks in.
pub fn forward(orientation: Quat) -> glm::Vec3 {
    let q: glm::Quat = orientation.into();
    glm::quat_rotate_vec3(&q, &glm::vec3(0.0, 0.0, VIEW_AXIS_SIGN))
}

/// Orientation that looks along `dir`, rolled so the horizon stays level.
pub fn look_rotation(dir: &glm::Vec3) -> Quat {
    let z = dir.normalize() * VIEW_AXIS_SIGN;
    let x = world_up().cross(&z).normalize();
    let y = z.cross(&x);
    let basis = glm::Mat3::from_columns(&[x, y, z]);
    glm::quat_normalize(&glm::mat3_to_quat(&basis)).into()
}

// Orbit

/// A camera circling a point. Stored as spherical coordinates and rebuilt into
/// a pose every frame, so orbiting never drifts off its radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    pub target: Vec3,
    /// Radians around the up axis. Never wrapped: turns accumulate, so orbit
    /// keys can ask for full spins.
    pub yaw: f32,
    /// Radians above the horizon.
    pub pitch: f32,
    pub distance: f32,
}

impl OrbitCamera {
    pub const MIN_DISTANCE: f32 = 10.0;

    /// Start orbiting whatever `pose` is looking at, `distance` ahead of it,
    /// without moving the camera.
    pub fn from_pose(pose: &CameraPose, distance: f32) -> Self {
        let eye: glm::Vec3 = pose.position.into();
        let target = eye + forward(pose.orientation) * distance;
        let offset = eye - target;
        Self {
            target: target.into(),
            yaw: offset.x.atan2(offset.z),
            pitch: (offset.y / distance).clamp(-1.0, 1.0).asin(),
            distance,
        }
    }

    /// From the target to the eye.
    fn offset(&self) -> glm::Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        glm::vec3(cp * sy, sp, cp * cy) * self.distance
    }

    /// What the camera circles.
    pub fn pivot(&self, skater: Option<Vec3>) -> Vec3 {
        skater.unwrap_or(self.target)
    }

    /// Where the camera sits and looks when orbiting `target`, which may
    /// differ from `self.target` when following a skater.
    pub fn pose_around(&self, target: Vec3, fov: f32) -> CameraPose {
        let offset = self.offset();
        let target: glm::Vec3 = target.into();
        CameraPose {
            position: (target + offset).into(),
            orientation: look_rotation(&-offset),
            fov,
        }
    }

    pub fn rotate(&mut self, d_yaw: f32, d_pitch: f32) {
        self.yaw += d_yaw;
        self.pitch = (self.pitch + d_pitch).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Shift `yaw` by whole turns to land nearest `reference`, so re-deriving
    /// an orbit from a pose does not undo the turns accumulated so far.
    pub fn with_yaw_near(mut self, reference: f32) -> Self {
        let turns = ((reference - self.yaw) / std::f32::consts::TAU).round();
        self.yaw += turns * std::f32::consts::TAU;
        self
    }

    /// Scale the distance, below 1 moves in, above 1 moves out.
    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance * factor).max(Self::MIN_DISTANCE);
    }

    /// Slide the target across the ground, relative to where the camera faces.
    pub fn pan(&mut self, right: f32, ahead: f32) {
        let (sy, cy) = self.yaw.sin_cos();
        let flat_forward = -glm::vec3(sy, 0.0, cy);
        let flat_right = flat_forward.cross(&world_up());
        let target: glm::Vec3 = self.target.into();
        self.target = (target + flat_right * right + flat_forward * ahead).into();
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f32) -> CameraPose {
        CameraPose {
            position: Vec3::new(x, 0.0, 0.0),
            orientation: Quat::IDENTITY,
            fov: 90.0,
        }
    }

    fn kf(time: f32, x: f32) -> Keyframe {
        Keyframe {
            time,
            pose: pose(x),
            tension: 0.0,
            orbit: None,
        }
    }

    fn orbit_kf(time: f32, yaw: f32) -> Keyframe {
        let orbit = OrbitCamera {
            target: Vec3::ZERO,
            yaw,
            pitch: 0.0,
            distance: 100.0,
        };
        Keyframe {
            time,
            pose: orbit.pose_around(orbit.target, 90.0),
            tension: 0.0,
            orbit: Some(orbit),
        }
    }

    fn radius(pose: CameraPose, centre: Vec3) -> f32 {
        (glm::Vec3::from(pose.position) - glm::Vec3::from(centre)).norm()
    }

    #[test]
    fn orbit_keys_sweep_round_the_target() {
        let mut path = CameraPath::new();
        path.push(orbit_kf(0.0, 0.0));
        path.push(orbit_kf(1.0, std::f32::consts::PI));
        // A world-space spline between opposite sides would pass through the
        // target; orbit space keeps the radius.
        let mid = path.sample(0.5, None).unwrap();
        assert!((radius(mid, Vec3::ZERO) - 100.0).abs() < 1e-2);
    }

    #[test]
    fn orbit_keys_can_spin_full_turns() {
        let mut path = CameraPath::new();
        path.push(orbit_kf(0.0, 0.0));
        path.push(orbit_kf(1.0, std::f32::consts::TAU));
        // Both keys sit at the same spot, but halfway through the spin the
        // camera is on the far side.
        let mid = path.sample(0.5, None).unwrap();
        assert!(
            (mid.position.z + 100.0).abs() < 1e-2,
            "got {:?}",
            mid.position
        );
    }

    #[test]
    fn orbit_keys_pivot_on_the_skater_when_there_is_one() {
        let mut path = CameraPath::new();
        path.push(orbit_kf(0.0, 0.0));
        path.push(orbit_kf(1.0, 1.0));
        let skater = Vec3::new(500.0, 20.0, -300.0);
        for t in [0.0, 0.3, 1.0] {
            let pose = path.sample(t, Some(skater)).unwrap();
            assert!((radius(pose, skater) - 100.0).abs() < 1e-2, "t={t}");
        }
    }

    #[test]
    fn yaw_near_keeps_accumulated_turns() {
        let orbit = OrbitCamera {
            target: Vec3::ZERO,
            yaw: 0.1,
            pitch: 0.0,
            distance: 100.0,
        };
        let near = orbit.with_yaw_near(4.0 * std::f32::consts::PI);
        assert!((near.yaw - (0.1 + 4.0 * std::f32::consts::PI)).abs() < 1e-4);
    }

    #[test]
    fn lerp_endpoints_return_the_inputs() {
        let a = CameraPose {
            fov: 60.0,
            ..pose(0.0)
        };
        let b = pose(10.0);
        assert_eq!(CameraPose::lerp(&a, &b, 0.0).position.x, 0.0);
        assert_eq!(CameraPose::lerp(&a, &b, 1.0).position.x, 10.0);
        assert_eq!(CameraPose::lerp(&a, &b, 0.5).fov, 75.0);
    }

    #[test]
    fn lerp_takes_the_short_way_round() {
        let a = CameraPose {
            orientation: Quat(0.0, 0.0, 0.0, 1.0),
            ..pose(0.0)
        };
        let b = CameraPose {
            orientation: Quat(0.0, 0.0, 0.0, -1.0),
            ..pose(0.0)
        };
        // Halfway between a rotation and itself is that same rotation.
        let mid = CameraPose::lerp(&a, &b, 0.5).orientation;
        assert!((mid.3.abs() - 1.0).abs() < 1e-5, "got {mid:?}");
    }

    #[test]
    fn sample_returns_none_with_one_frame() {
        let mut path = CameraPath::new();
        path.push(kf(0.0, 0.0));
        assert!(path.sample(0.0, None).is_none());
    }

    #[test]
    fn sample_past_end_returns_none() {
        let mut path = CameraPath::new();
        path.push(kf(0.0, 0.0));
        path.push(kf(1.0, 1.0));
        assert!(path.sample(2.0, None).is_none());
    }

    #[test]
    fn sample_at_start_returns_first_frame() {
        let mut path = CameraPath::new();
        path.push(kf(0.0, 0.0));
        path.push(kf(1.0, 10.0));
        let pose = path.sample(0.0, None).unwrap();
        assert_eq!(pose.position.x, 0.0);
        assert_eq!(pose.fov, 90.0);
    }

    #[test]
    fn speed_is_continuous_across_uneven_keys() {
        let mut path = CameraPath::new();
        path.push(kf(0.0, 0.0));
        path.push(kf(1.0, 2.0));
        path.push(kf(5.0, 3.0));
        path.push(kf(6.0, 10.0));

        let x = |t: f32| path.sample(t, None).unwrap().position.x;
        let h = 1e-3;
        let before = (x(1.0) - x(1.0 - h)) / h;
        let after = (x(1.0 + h) - x(1.0)) / h;
        assert!(
            (before - after).abs() < 0.05,
            "before {before}, after {after}"
        );
    }

    #[test]
    fn look_rotation_faces_the_given_direction() {
        let dir = glm::vec3(3.0, -1.0, 2.0).normalize();
        let got = forward(look_rotation(&dir));
        assert!((got - dir).norm() < 1e-5, "got {got:?}");
    }

    #[test]
    fn orbit_from_pose_keeps_the_camera_in_place() {
        let start = CameraPose {
            position: Vec3::new(10.0, 50.0, -20.0),
            orientation: look_rotation(&glm::vec3(1.0, -0.3, 0.5)),
            fov: 90.0,
        };
        let orbit = OrbitCamera::from_pose(&start, 300.0);
        let pose = orbit.pose_around(orbit.target, start.fov);

        let moved = glm::Vec3::from(pose.position) - glm::Vec3::from(start.position);
        assert!(moved.norm() < 1e-2, "moved by {moved:?}");
        let turned = forward(pose.orientation) - forward(start.orientation);
        assert!(turned.norm() < 1e-4, "turned by {turned:?}");
    }

    #[test]
    fn orbit_rotation_keeps_the_radius() {
        let mut orbit = OrbitCamera {
            target: Vec3::new(5.0, 0.0, 5.0),
            yaw: 0.0,
            pitch: 0.2,
            distance: 100.0,
        };
        orbit.rotate(2.0, 5.0);
        let eye: glm::Vec3 = orbit.pose_around(orbit.target, 90.0).position.into();
        let radius = (eye - glm::Vec3::from(orbit.target)).norm();
        assert!((radius - 100.0).abs() < 1e-3, "radius {radius}");
        assert!(orbit.pitch <= MAX_PITCH);
    }

    #[test]
    fn push_maintains_sort_order() {
        let mut path = CameraPath::new();
        path.push(kf(2.0, 2.0));
        path.push(kf(0.0, 0.0));
        path.push(kf(1.0, 1.0));
        assert_eq!(path.frames()[0].time, 0.0);
        assert_eq!(path.frames()[1].time, 1.0);
        assert_eq!(path.frames()[2].time, 2.0);
    }
}
