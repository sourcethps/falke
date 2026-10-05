use std::sync::Mutex;

use crate::{
    game,
    replay::{Replay, ReplayState},
};

/// The active replay. Set by `start`, retrieved by `stop`. None when not
/// recording.
static ACTIVE_REPLAY: Mutex<Option<Replay>> = Mutex::new(None);

/// Hand a prepared Replay to the capture pipeline. Called by `RecordingSession::start`.
pub fn start(replay: Replay) {
    *ACTIVE_REPLAY.lock().unwrap() = Some(replay);
}

/// Take the Replay back from the capture pipeline. Called by `RecordingSession::stop`.
/// Returns `None` if not currently recording.
pub fn stop() -> Option<Replay> {
    ACTIVE_REPLAY.lock().unwrap().take()
}

/// True while a recording is in progress.
pub fn is_recording() -> bool {
    ACTIVE_REPLAY.lock().map(|g| g.is_some()).unwrap_or(false)
}

pub unsafe fn on_endscene(playback: Option<&ReplayState>) {
    let mut replay_guard = ACTIVE_REPLAY.lock().unwrap();

    if replay_guard.is_none() && playback.is_none() {
        return;
    }

    let Some(game) = game::current() else {
        return;
    };

    if let Some(replay) = replay_guard.as_mut() {
        let mut frame = ReplayState::default();
        unsafe { game.capture_frame(&mut frame) };

        if let Err(e) = replay.write_frame(&frame) {
            log::error!("failed to serialize replay frame: {e}");
        } else {
            log_frame(replay, &frame);
        }
    }

    // Playback.
    if let Some(frame) = playback {
        unsafe { game.apply_frame(frame) };
    }
}

fn log_frame(replay: &Replay, frame: &ReplayState) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let pos_str = frame
        .skaters
        .first()
        .map(|s| {
            let p = s.world_pos;
            format!(" pos=({:.1},{:.1},{:.1})", p[0], p[1], p[2])
        })
        .unwrap_or_default();
    let cam_str = frame
        .camera
        .map(|c| {
            let p = c.position;
            format!(" cam=({:.1},{:.1},{:.1})", p.x, p.y, p.z)
        })
        .unwrap_or_else(|| " cam=none".into());
    log::debug!(
        "[record] frame {}{}{}",
        replay.frame_count(),
        pos_str,
        cam_str
    );
}
