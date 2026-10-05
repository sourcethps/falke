use anyhow::Result;

use crate::{camera::CameraPose, game::Game, replay::ReplayState};

pub struct Thps4;

impl Game for Thps4 {
    fn name(&self) -> &'static str {
        "THPS4"
    }

    unsafe fn install_hooks(&self) -> Result<()> {
        log::warn!("[THPS4] unimplemented");
        Ok(())
    }

    unsafe fn capture_frame(&self, _frame: &mut ReplayState) {
        // TODO: for each live skater, push a `SkaterState`:
    }

    fn in_freecam(&self) -> bool {
        // TODO: read the game's freecam flag. Returning false keeps the UI
        // hidden, which is the honest answer while nothing else is wired up.
        false
    }

    fn camera(&self) -> Option<CameraPose> {
        // TODO: read the camera the user flies — position, orientation and fov.
        None
    }

    fn set_camera(&self, _pose: &CameraPose) {
        // TODO: write the pose back into that camera.
    }

    unsafe fn apply_frame(&self, _frame: &ReplayState) {
        // TODO
    }
}
