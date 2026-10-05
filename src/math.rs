use nalgebra_glm as glm;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// The game's layout, and what gets serialised.
    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    pub fn from_array(v: [f32; 3]) -> Self {
        Self::new(v[0], v[1], v[2])
    }
}

impl From<Vec3> for glm::Vec3 {
    fn from(v: Vec3) -> Self {
        glm::vec3(v.x, v.y, v.z)
    }
}

impl From<glm::Vec3> for Vec3 {
    fn from(v: glm::Vec3) -> Self {
        Vec3 {
            x: v.x,
            y: v.y,
            z: v.z,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat(pub f32, pub f32, pub f32, pub f32);

impl Quat {
    pub const IDENTITY: Self = Self(0.0, 0.0, 0.0, 1.0);

    /// The game's layout, and what gets serialised.
    pub fn to_array(self) -> [f32; 4] {
        [self.0, self.1, self.2, self.3]
    }

    pub fn from_array(q: [f32; 4]) -> Self {
        Self(q[0], q[1], q[2], q[3])
    }
}

impl From<Quat> for glm::Quat {
    fn from(q: Quat) -> Self {
        glm::make_quat(&[q.0, q.1, q.2, q.3])
    }
}

impl From<glm::Quat> for Quat {
    fn from(q: glm::Quat) -> Self {
        let qv = q.as_vector();
        Quat(qv.x, qv.y, qv.z, qv.w)
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

pub fn lerp_arr<const N: usize>(a: [f32; N], b: [f32; N], t: f32) -> [f32; N] {
    std::array::from_fn(|i| lerp(a[i], b[i], t))
}
