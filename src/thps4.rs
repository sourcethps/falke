use std::{
    mem::transmute,
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result};
use nalgebra_glm as glm;
use retour::static_detour;

use thps_rs::thps4::{
    CSkater, GfxCamera, MthMatrix, MthVector, SIG_B4_DO_FREECAM_LOGIC, gfx_camera_matrix,
    gfx_camera_pos, gfx_camera_set_matrix, gfx_camera_set_pos,
};

use crate::{camera::CameraPose, game::Game, math::Vec3, replay::ReplayState};

static_detour! {
    static DoFreecamLogicHook: unsafe extern "C" fn(*mut CSkater);
}

static SKATER: AtomicUsize = AtomicUsize::new(0);

const FOV: f32 = 90.0;

pub struct Thps4;

impl Game for Thps4 {
    fn name(&self) -> &'static str {
        "THPS4"
    }

    unsafe fn install_hooks(&self) -> Result<()> {
        unsafe { install_hooks() }
    }

    unsafe fn capture_frame(&self, _frame: &mut ReplayState) {
        // TODO: for each live skater, push a `SkaterState`:
    }

    fn in_freecam(&self) -> bool {
        let Some(skater) = skater() else {
            return false;
        };
        unsafe { (*skater).view_mode > 0 }
    }

    fn camera(&self) -> Option<CameraPose> {
        let camera = camera_obj()?;
        let (pos, matrix) = unsafe { (gfx_camera_pos(camera), gfx_camera_matrix(camera)) };

        let mat4: glm::Mat4 = glm::make_mat4(&mat_to_array(&matrix));
        let mat3: glm::Mat3 = glm::mat4_to_mat3(&mat4);
        let quat: glm::Quat = glm::quat_normalize(&glm::mat3_to_quat(&mat3));

        Some(CameraPose {
            position: Vec3::new(pos.x, pos.y, pos.z),
            orientation: quat.into(),
            fov: FOV,
        })
    }

    fn set_camera(&self, pose: &CameraPose) {
        let Some(camera) = camera_obj() else {
            return;
        };

        let rot: glm::Quat = pose.orientation.into();
        let matrix = mat_from_slice(glm::quat_to_mat4(&rot).as_slice());

        unsafe {
            gfx_camera_set_matrix(camera, &matrix);
            // Read-modify-write so the position's w stays whatever the game
            // put there; only x, y and z are ours.
            let mut pos = gfx_camera_pos(camera);
            pos.x = pose.position.x;
            pos.y = pose.position.y;
            pos.z = pose.position.z;
            gfx_camera_set_pos(camera, &pos);
        }
    }

    unsafe fn apply_frame(&self, _frame: &ReplayState) {
        // TODO
    }
}

// Live object access
fn skater() -> Option<*mut CSkater> {
    let skater = SKATER.load(Ordering::Relaxed) as *mut CSkater;
    (!skater.is_null()).then_some(skater)
}

fn camera_obj() -> Option<GfxCamera> {
    let camera = unsafe { (*skater()?).camera } as GfxCamera;
    (camera != 0).then_some(camera)
}

fn mat_to_array(m: &MthMatrix) -> [f32; 16] {
    [
        m.x.x, m.x.y, m.x.z, m.x.w, m.y.x, m.y.y, m.y.z, m.y.w, m.z.x, m.z.y, m.z.z, m.z.w, m.w.x,
        m.w.y, m.w.z, m.w.w,
    ]
}

fn mat_from_slice(a: &[f32]) -> MthMatrix {
    let row = |i: usize| MthVector {
        x: a[i],
        y: a[i + 1],
        z: a[i + 2],
        w: a[i + 3],
    };
    MthMatrix {
        x: row(0),
        y: row(4),
        z: row(8),
        w: row(12),
    }
}

// Hook installation
unsafe fn install_hooks() -> Result<()> {
    unsafe {
        let better4 = toy_arms::internal::module::Module::from_name("better4.dll")
            .context("failed to find better4.dll — is Better4 loaded?")?;
        log::debug!("[addr] better4.dll base: 0x{:x}", better4.base_address);

        let addr_do_freecam_logic = better4
            .find_pattern(SIG_B4_DO_FREECAM_LOGIC)
            .context("failed to find do_freecam_logic")?
            + better4.base_address;
        log::debug!("[addr] do_freecam_logic: 0x{:x}", addr_do_freecam_logic);
        DoFreecamLogicHook
            .initialize(transmute(addr_do_freecam_logic), hk_do_freecam_logic)
            .context("failed to init do_freecam_logic hook")?
            .enable()
            .context("failed to enable do_freecam_logic hook")?;
        log::info!("[THPS4] do_freecam_logic hook installed");

        Ok(())
    }
}

// Detour handlers
fn hk_do_freecam_logic(skater: *mut CSkater) {
    if SKATER.swap(skater as usize, Ordering::Relaxed) == 0 {
        unsafe {
            log::debug!(
                "[THPS4] first freecam tick: CSkater=0x{:x} view_mode={} CSkaterCam=0x{:x}",
                skater as usize,
                (*skater).view_mode,
                (*skater).camera as usize,
            );
        }
    }

    unsafe { DoFreecamLogicHook.call(skater) };
}
