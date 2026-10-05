//! THUG2 / THUGPro hook installation and per-frame capture logic.

use std::{
    mem::transmute,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use anyhow::{Context, Result};
use nalgebra_glm as glm;
use retour::static_detour;

use thps_rs::thug2::{
    ADDR_G_COMPOSITE_OBJECT_MANAGER, ADDR_GET_ACTIVE_CAMERA, ADDR_GET_BONE_WORLD_POSITION,
    ADDR_MODEL_RENDER, ADDR_PROCESS_ALL_OBJECTS, ADDR_SKELETON_UPDATE, CCompositeObject, CObject,
    CRC_MODEL_COMPONENT, CRC_SKELETON_COMPONENT, GetBoneWorldPositionFn, GetComponentFn,
    MODEL_COMPONENT_MODEL_OFFSET, MODEL_RENDER_FN, ModelRenderFn, ObjectType, ProcessAllObjectsFn,
    SET_SCREEN_ANGLE_FN, SIG_GET_COMPONENT, SIG_SET_SCREEN_ANGLE, SIG_VIEWER_TRANSLATE_LOCAL,
    SetScreenAngleFn, read_screen_angle, write_screen_angle,
};

use thps_rs::crc;
use windows::Win32::UI::WindowsAndMessaging::ShowCursor;

use crate::{
    camera::CameraPose,
    game::Game,
    math::Vec3,
    replay::{ReplayState, SkaterState},
};

static_detour! {
    pub static ViewerTranslateLocalHook: unsafe extern fn(*mut CCompositeObject, *mut usize) -> ();
    static SkeletonUpdateHook: unsafe extern "thiscall" fn(*const usize, *mut [[f32;4];50], *mut [[f32;4];50]);
    static GetActiveCameraHook: unsafe extern fn(i32) -> usize;
}

// Per-frame skeleton pose state
unsafe impl Send for SkeletonState {}
unsafe impl Sync for SkeletonState {}
struct SkeletonState {
    quaternions: Option<*mut [[f32; 4]; 50]>,
    translations: Option<*mut [[f32; 4]; 50]>,
}
static SKELETON_STATE: Mutex<SkeletonState> = Mutex::new(SkeletonState {
    quaternions: None,
    translations: None,
});

// Resolved function pointers

static GET_COMPONENT_FN: OnceLock<GetComponentFn> = OnceLock::new();
static GET_BONE_WORLD_POSITION_FN: OnceLock<GetBoneWorldPositionFn> = OnceLock::new();

// Capture pipeline

/// The freecam viewer object, discovered by `hk_viewer_translate_local`.
static CAMERA_OBJ: AtomicUsize = AtomicUsize::new(0);

/// The camera the game is currently rendering through.
static ACTIVE_CAMERA: AtomicUsize = AtomicUsize::new(0);

// Game backend

pub struct Thug2;

impl Game for Thug2 {
    fn name(&self) -> &'static str {
        "THUG2 / THUGPro"
    }

    unsafe fn install_hooks(&self) -> Result<()> {
        unsafe { install_hooks() }
    }

    unsafe fn capture_frame(&self, frame: &mut ReplayState) {
        unsafe {
            // Prefer the camera the game is rendering through; fall back to the
            // freecam viewer only if GetActiveCamera has not fired yet.
            let cam_obj = match ACTIVE_CAMERA.load(Ordering::Relaxed) {
                0 => CAMERA_OBJ.load(Ordering::Relaxed),
                active => active,
            };
            if cam_obj != 0 {
                frame.camera = Some(read_camera(cam_obj));
            }

            // The callback fills `frame.skaters` through ProcessAllObjects'
            // user-data parameter, so it touches no mutex.
            let manager = *(ADDR_G_COMPOSITE_OBJECT_MANAGER as *const usize);
            let ctx = frame as *mut ReplayState as usize;
            call_process_all_objects(manager, falke_process_all_objects, ctx);
        }
    }

    fn in_freecam(&self) -> bool {
        thps_rs::thug2::in_freecam()
    }

    /// The freecam viewer, what the user flies.
    fn camera(&self) -> Option<CameraPose> {
        match CAMERA_OBJ.load(Ordering::Relaxed) {
            0 => None,
            obj => Some(read_camera(obj)),
        }
    }

    fn set_camera(&self, pose: &CameraPose) {
        let obj = CAMERA_OBJ.load(Ordering::Relaxed);
        if obj != 0 {
            write_camera(obj, pose);
        }
    }

    unsafe fn apply_frame(&self, frame: &ReplayState) {
        // Single-skater for now, more means looping this body.
        let Some(skater) = frame.skaters.first() else {
            return;
        };

        unsafe {
            let manager = *(ADDR_G_COMPOSITE_OBJECT_MANAGER as *const usize);
            let mut targets: Vec<(usize, usize)> = Vec::with_capacity(4);
            call_process_all_objects(
                manager,
                collect_render_targets,
                &mut targets as *mut Vec<(usize, usize)> as usize,
            );

            // Build full render matrix.
            let mut render_matrix = skater.world_matrix;
            render_matrix[12] = skater.world_pos[0];
            render_matrix[13] = skater.world_pos[1];
            render_matrix[14] = skater.world_pos[2];
            render_matrix[15] = 1.0;

            // Get the live skeleton's quat/trans array pointers (valid for game lifetime).
            let (quat_ptr, trans_ptr) = {
                let skel = SKELETON_STATE.lock().unwrap();
                (skel.quaternions, skel.translations)
            };

            for &(model_ptr, skeleton_ptr) in targets.iter() {
                if model_ptr == 0 || skeleton_ptr == 0 {
                    continue;
                }

                // Any target here has a live skeleton, so SkeletonUpdate has run
                // and these pointers are set. Skip rather than guess if not.
                let (Some(qp), Some(tp)) = (quat_ptr, trans_ptr) else {
                    continue;
                };

                // Write replay bone poses, call SkeletonUpdate to recompute bone
                // matrices from them, render with no_anim=true, then restore.
                let saved_q = *qp;
                let saved_t = *tp;
                *qp = skater.bone_quats;
                *tp = skater.bone_trans;
                SkeletonUpdateHook.call(skeleton_ptr as *const usize, qp, tp);
                if let Some(render) = MODEL_RENDER_FN.get() {
                    render(
                        model_ptr as *const usize,
                        &render_matrix,
                        true,
                        skeleton_ptr,
                    );
                }
                *qp = saved_q;
                *tp = saved_t;
            }
        }
    }
}

// Hook installation
unsafe fn install_hooks() -> Result<()> {
    unsafe {
        let main_module = toy_arms::internal::module::Module::from_name("THUGPro.exe")
            .or_else(|| toy_arms::internal::module::Module::from_name("THUG2.exe"))
            .context("failed to find THUG2/THUGPro main module")?;

        let addr_translate_local = main_module
            .find_pattern(SIG_VIEWER_TRANSLATE_LOCAL)
            .context("failed to find CViewer::s_translate_local")?
            + main_module.base_address;
        log::debug!(
            "[addr] CViewer::s_translate_local: 0x{:x}",
            addr_translate_local
        );
        ViewerTranslateLocalHook
            .initialize(transmute(addr_translate_local), hk_viewer_translate_local)
            .context("failed to init CViewer::s_translate_local hook")?
            .enable()
            .context("failed to enable CViewer::s_translate_local hook")?;

        let addr_set_screen_angle = main_module
            .find_pattern(SIG_SET_SCREEN_ANGLE)
            .context("failed to find CViewportManager::SetScreenAngle")?
            + main_module.base_address;
        log::debug!(
            "[addr] CViewportManager::SetScreenAngle: 0x{:x}",
            addr_set_screen_angle
        );
        let func: SetScreenAngleFn = transmute(addr_set_screen_angle);
        if SET_SCREEN_ANGLE_FN.set(func).is_err() {
            log::error!("[func] SetScreenAngle failed to initialize");
        }

        let addr_get_component = main_module
            .find_pattern(SIG_GET_COMPONENT)
            .map(|off| off + main_module.base_address)
            .or_else(|| {
                toy_arms::internal::module::Module::from_name("thugpro.dll").and_then(|m| {
                    m.find_pattern(SIG_GET_COMPONENT)
                        .map(|off| off + m.base_address)
                })
            });
        match addr_get_component {
            Some(addr) => {
                log::debug!("[addr] CObject::GetComponent: 0x{:x}", addr);
                let func: GetComponentFn = transmute(addr);
                if GET_COMPONENT_FN.set(func).is_err() {
                    log::error!("[func] GetComponent already initialized");
                }
            }
            None => log::warn!(
                "[addr] CObject::GetComponent not found in any module — bone world pos disabled"
            ),
        }

        log::debug!("[addr] GetActiveCamera: 0x{:x}", ADDR_GET_ACTIVE_CAMERA);
        GetActiveCameraHook
            .initialize(transmute(ADDR_GET_ACTIVE_CAMERA), hk_get_active_camera)
            .context("failed to init GetActiveCamera hook")?
            .enable()
            .context("failed to enable GetActiveCamera hook")?;

        log::debug!("[addr] Skeleton::Update: 0x{:x}", ADDR_SKELETON_UPDATE);
        SkeletonUpdateHook
            .initialize(transmute(ADDR_SKELETON_UPDATE), hk_skeleton_update)
            .context("failed to init Skeleton::Update hook")?
            .enable()
            .context("failed to enable Skeleton::Update hook")?;

        log::debug!(
            "[addr] SkeletonComponent::GetBoneWorldPosition: 0x{:x}",
            ADDR_GET_BONE_WORLD_POSITION
        );
        let func: GetBoneWorldPositionFn = transmute(ADDR_GET_BONE_WORLD_POSITION);
        if GET_BONE_WORLD_POSITION_FN.set(func).is_err() {
            log::error!("[func] GetBoneWorldPosition failed to initialize");
        }

        log::debug!("[addr] ModelRender: 0x{:x}", ADDR_MODEL_RENDER);
        let func: ModelRenderFn = transmute(ADDR_MODEL_RENDER);
        if MODEL_RENDER_FN.set(func).is_err() {
            log::error!("[func] ModelRender failed to initialize");
        }

        Ok(())
    }
}

// Camera operations.
fn read_camera(cam_obj: usize) -> CameraPose {
    let camera = unsafe { *(cam_obj as *const CCompositeObject) };

    let mat4: glm::Mat4 = glm::make_mat4(&camera.matrix);
    let mat3: glm::Mat3 = glm::mat4_to_mat3(&mat4);
    let quat: glm::Quat = glm::quat_normalize(&glm::mat3_to_quat(&mat3));

    CameraPose {
        position: Vec3::from_array(camera.position),
        orientation: quat.into(),
        fov: read_screen_angle(),
    }
}

/// Write a pose into a camera object.
fn write_camera(cam_obj: usize, pose: &CameraPose) {
    let rot: glm::Quat = pose.orientation.into();
    let mat4 = glm::quat_to_mat4(&rot);
    unsafe {
        let obj = cam_obj as *mut CCompositeObject;
        (*obj).position = pose.position.to_array();
        (*obj).matrix.copy_from_slice(mat4.as_slice());
    }
    write_screen_angle(pose.fov);
}

// Detour handlers

fn hk_viewer_translate_local(obj: *mut CCompositeObject, vec: *mut usize) {
    // First sighting only: this is where the freecam camera object becomes
    // known, and where the UI first has something to fly.
    if CAMERA_OBJ.swap(obj as usize, Ordering::Relaxed) == 0 {
        log::debug!("[addr] Camera: 0x{:x}", obj as usize);
        unsafe { ShowCursor(true) };
    }
    unsafe { ViewerTranslateLocalHook.call(obj, vec) }
}

fn hk_get_active_camera(index: i32) -> usize {
    let camera = unsafe { GetActiveCameraHook.call(index) };
    if camera != 0 && ACTIVE_CAMERA.swap(camera, Ordering::Relaxed) != camera {
        log::debug!("[addr] active camera: 0x{camera:x} (viewport {index})");
    }
    camera
}

fn hk_skeleton_update(this: *const usize, pquat: *mut [[f32; 4]; 50], ptrans: *mut [[f32; 4]; 50]) {
    unsafe { SkeletonUpdateHook.call(this, pquat, ptrans) };
    if let Ok(mut state) = SKELETON_STATE.lock() {
        state.quaternions = Some(pquat);
        state.translations = Some(ptrans);
    }
}

// Frame capture

unsafe fn call_process_all_objects(this: usize, cb: thps_rs::thug2::ObjectCallback, data: usize) {
    let func: ProcessAllObjectsFn = unsafe { transmute(ADDR_PROCESS_ALL_OBJECTS) };
    unsafe { func(this, cb, data) };
}

/// Collects (model_ptr, skeleton_ptr) for every live skater into the
/// `Vec<(usize, usize)>` handed in through `data`.
unsafe extern "C" fn collect_render_targets(object: *mut CObject, data: usize) {
    if object.is_null() || data == 0 {
        return;
    }
    let obj = unsafe { *object };
    if obj.m_type != ObjectType::Skater || obj.m_type == ObjectType::Ped {
        return;
    }

    let Some(get_component) = GET_COMPONENT_FN.get() else {
        return;
    };

    let model_component = unsafe { get_component(object as usize, CRC_MODEL_COMPONENT) };
    if model_component == 0 {
        return;
    }
    let model_ptr = unsafe { *((model_component + MODEL_COMPONENT_MODEL_OFFSET) as *const usize) };

    let skeleton_component = unsafe { get_component(object as usize, CRC_SKELETON_COMPONENT) };
    if skeleton_component == 0 {
        return;
    }
    let skeleton_ptr = unsafe { *((skeleton_component + 0x18) as *const usize) };

    unsafe { &mut *(data as *mut Vec<(usize, usize)>) }.push((model_ptr, skeleton_ptr));
}

/// Captures every live skater into the `ReplayState` handed in through `data`.
unsafe extern "C" fn falke_process_all_objects(object: *mut CObject, data: usize) {
    if object.is_null() || data == 0 {
        return;
    }
    let obj = unsafe { *object };
    if obj.m_type == ObjectType::Skater {
        let Some(skater) = (unsafe { capture_skater_state(object) }) else {
            log::warn!("skater seen but skeleton state not ready yet");
            return;
        };
        unsafe { &mut *(data as *mut ReplayState) }
            .skaters
            .push(skater);
    }
}

unsafe fn capture_skater_state(object: *mut CObject) -> Option<SkaterState> {
    let get_component = GET_COMPONENT_FN.get().or_else(|| {
        log::warn!("[capture] GET_COMPONENT_FN not initialized");
        None
    })?;
    let get_bone_world_pos = GET_BONE_WORLD_POSITION_FN.get().or_else(|| {
        log::warn!("[capture] GET_BONE_WORLD_POSITION_FN not initialized");
        None
    })?;

    let component_addr = unsafe { get_component(object as usize, CRC_SKELETON_COMPONENT) };
    if component_addr == 0 {
        log::warn!("[capture] GetComponent returned null — object has no SkeletonComponent");
        return None;
    }
    let component = component_addr as *mut usize;

    let world_matrix = unsafe { (*(object as *mut CCompositeObject)).matrix };

    let (quaternions, translations) = {
        let state = SKELETON_STATE.lock().ok().or_else(|| {
            log::warn!("[capture] failed to lock SKELETON_STATE");
            None
        })?;
        let quats = unsafe {
            *(state.quaternions.or_else(|| {
                log::warn!(
                    "[capture] quaternions pointer is None — SkeletonUpdateHook has not fired yet"
                );
                None
            })?)
        };
        let trans = unsafe {
            *(state.translations.or_else(|| {
                log::warn!(
                    "[capture] translations pointer is None — SkeletonUpdateHook has not fired yet"
                );
                None
            })?)
        };
        (quats, trans)
    };

    let mut world_pos = [0.0f32; 4];
    unsafe {
        get_bone_world_pos(
            component as *const usize,
            crc::generate_crc("control_root") as usize,
            &mut world_pos,
        );
    }

    static LOGGED: AtomicBool = AtomicBool::new(false);
    if !LOGGED.swap(true, Ordering::Relaxed) {
        log::debug!("[capture] bone[0] quat={:?}", quaternions[0]);
        log::debug!("[capture] bone[0] trans={:?}", translations[0]);
    }

    Some(SkaterState {
        world_pos: [world_pos[0], world_pos[1], world_pos[2]],
        world_matrix,
        bone_quats: quaternions,
        bone_trans: translations,
    })
}
