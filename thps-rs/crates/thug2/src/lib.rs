use std::sync::OnceLock;

// Byte-pattern signatures

pub const SIG_VIEWER_TRANSLATE_LOCAL: &str = "83 EC 40 56 8B 74 24 48 8D 46 6C 50 8D 4C 24 08 ? ? ? ? ? D9 46 58 8B 46 54 D9 44 24 24 8B 4E 4C 89";
pub const SIG_SET_SCREEN_ANGLE: &str =
    "D9 05 20 D5 66 00 D9 44 24 04 DA E9 DF E0 F6 C4 44 ? ? A1 48 13";
pub const SIG_GET_COMPONENT: &str = "8B 81 00 01 00 00 85 C0 74 ? 8B 4C 24 ?";

// Known absolute addresses

pub const ADDR_S_SCREEN_ANGLE: usize = 0x00701344;
pub const ADDR_FREECAM: usize = 0x007CE4A4;
pub const ADDR_G_COMPOSITE_OBJECT_MANAGER: usize = 0x006f0fb8;
pub const ADDR_SKELETON_UPDATE: usize = 0x004A4240;
pub const ADDR_GET_BONE_SCALE: usize = 0x004A2D20;
pub const ADDR_GET_BONE_WORLD_POSITION: usize = 0x00432050;
pub const ADDR_PROCESS_ALL_OBJECTS: usize = 0x00461cc0;
pub const ADDR_MODEL_RENDER: usize = 0x00493090;
pub const ADDR_GET_ACTIVE_CAMERA: usize = 0x004A09C0;

// CRC constants

pub const CRC_SKELETON_COMPONENT: usize = 0x222756d5;
pub const CRC_MODEL_COMPONENT: usize = 0x286a8d26;

// Offset within ModelComponent to the model pointer.
pub const MODEL_COMPONENT_MODEL_OFFSET: usize = 0x208;

// Object system

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CObject {
    _pad: [u8; 0x14],
    pub id: u32,
    pub m_type: ObjectType,
}

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ObjectType {
    Undefined = 0x0b3daf3e,
    Skater = 0xbaf9b341,
    Composite = 0xcdeb9229,
    GameObj = 0xbc0d91d8,
    Car = 0x5c0312eb,
    Ped = 0xd2db7f97,
}

// Composite object (camera / skater)

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CCompositeObject {
    _pad_001: [u8; 76],
    pub position: [f32; 3],
    _pad_002: [u8; 20],
    pub matrix: [f32; 16],
}

// Function ABI typedefs

pub type ObjectCallback = unsafe extern "C" fn(object: *mut CObject, data: usize);
pub type ProcessAllObjectsFn =
    unsafe extern "thiscall" fn(this: usize, process: ObjectCallback, data: usize);
pub type GetComponentFn = unsafe extern "thiscall" fn(usize, usize) -> usize;
pub type GetBoneScaleFn = unsafe extern "thiscall" fn(*const usize, usize, *mut [f32; 4]);
pub type GetBoneWorldPositionFn =
    unsafe extern "thiscall" fn(*const usize, usize, *mut [f32; 4]) -> bool;
pub type SetScreenAngleFn = unsafe extern "C" fn(f32);
pub type ModelRenderFn = unsafe extern "thiscall" fn(
    model: *const usize,
    matrix: *const [f32; 16],
    no_anim: bool,
    skeleton: usize,
);

pub static SET_SCREEN_ANGLE_FN: OnceLock<SetScreenAngleFn> = OnceLock::new();
pub static MODEL_RENDER_FN: OnceLock<ModelRenderFn> = OnceLock::new();

pub fn read_screen_angle() -> f32 {
    unsafe { *(ADDR_S_SCREEN_ANGLE as *const f32) }
}

pub fn write_screen_angle(angle: f32) -> Option<()> {
    let func = SET_SCREEN_ANGLE_FN.get()?;
    unsafe { func(angle) };
    Some(())
}

pub fn in_freecam() -> bool {
    unsafe { *(ADDR_FREECAM as *const bool) }
}
