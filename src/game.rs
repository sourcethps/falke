use std::sync::OnceLock;

use anyhow::{Context, Result, bail};

use crate::{camera::CameraPose, replay::ReplayState};

pub trait Game: Sync {
    /// Human-readable name, for logs.
    fn name(&self) -> &'static str;

    /// Install this game's hooks and resolve its function pointers.
    /// Called once at startup, after detection.
    unsafe fn install_hooks(&self) -> Result<()>;

    /// Fill `frame` with everything this tick should record.
    unsafe fn capture_frame(&self, frame: &mut ReplayState);

    /// Apply a recorded `frame` into the live scene.
    unsafe fn apply_frame(&self, frame: &ReplayState);

    /// True while the game is in its free-flying camera state.
    fn in_freecam(&self) -> bool;

    /// The pose of the camera the user flies, or `None` before the backend has
    /// found one.
    fn camera(&self) -> Option<CameraPose>;

    /// Push a pose into the camera the user flies. No-op if there is none yet.
    fn set_camera(&self, pose: &CameraPose);
}

static GAME: OnceLock<&'static dyn Game> = OnceLock::new();

/// Pick a backend from the host executable's file name.
fn detect() -> Result<&'static dyn Game> {
    let exe = std::env::current_exe().context("failed to read current executable path")?;
    let name = exe
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .context("executable path has no file name")?;

    match name.as_str() {
        "thugpro.exe" | "thug2.exe" => Ok(&crate::thug2::Thug2),
        "thps4.exe" | "better4.exe" => Ok(&crate::thps4::Thps4),
        other => bail!("unsupported game executable: {other}"),
    }
}

/// Detect the host game and install its hooks.
pub fn init() -> Result<()> {
    let game = detect()?;
    let _ = GAME.set(game);
    log::info!("detected game: {}", game.name());
    unsafe { game.install_hooks() }.context("failed to install game-specific hooks")
}

/// The detected backend, or `None` before `init()` has run.
pub fn current() -> Option<&'static dyn Game> {
    GAME.get().copied()
}

pub fn in_freecam() -> bool {
    current().is_some_and(|g| g.in_freecam())
}

/// The pose of the camera the user flies, or `None` if there is none yet.
pub fn camera() -> Option<CameraPose> {
    current().and_then(|g| g.camera())
}

/// Push a pose into the camera the user flies.
pub fn set_camera(pose: &CameraPose) {
    if let Some(g) = current() {
        g.set_camera(pose);
    }
}
