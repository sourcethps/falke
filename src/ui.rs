use egui::{
    Align2, Color32, Context as eguiContext, DragValue, FontId, Pos2, Rect, Sense, TopBottomPanel,
    Vec2,
};
use std::{
    sync::{Mutex, OnceLock},
    time::Instant,
};
use windows::Win32::UI::WindowsAndMessaging::ShowCursor;

use crate::{
    camera::{CameraPath, CameraPose, Keyframe, OrbitCamera},
    capture,
    falke::version_string,
    game,
    math::Vec3,
    replay::{LoadedReplay, ReplayState},
};

pub static UI_SYSTEM: OnceLock<Mutex<UiSystem>> = OnceLock::new();

/// Narrowest the zoom window may get, in seconds.
const MIN_VISIBLE_DURATION: f32 = 0.5;

/// Fov for a pose built before the game has told us what the camera uses.
const DEFAULT_FOV: f32 = 90.0;

/// How far ahead of the camera the orbit pivot lands when orbit mode starts.
const DEFAULT_ORBIT_DISTANCE: f32 = 300.0;

/// Radians of orbit per pixel dragged.
const ORBIT_ROTATE_SPEED: f32 = 0.005;

/// Target movement per pixel dragged, as a fraction of the orbit distance, so
/// panning feels the same close up and far away.
const ORBIT_PAN_SPEED: f32 = 0.002;

/// Zoom per point of wheel scroll, one notch is roughly 50 points.
const ORBIT_ZOOM_SPEED: f32 = 0.002;

/// Owns the camera path and timeline viewport state.
pub struct Timeline {
    pub path: CameraPath,
    pub selected_index: Option<usize>,
    pub dragging_index: Option<usize>,
    pub total_duration: f32,
    pub visible_duration: f32,
    pub scroll_offset: f32,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            path: CameraPath::new(),
            selected_index: None,
            dragging_index: None,
            total_duration: 90.0,
            visible_duration: 30.0,
            scroll_offset: 0.0,
        }
    }
}

impl Timeline {
    /// Add a keyframe, sort the path, and select the last frame.
    pub fn add_keyframe(&mut self, kf: Keyframe) {
        self.path.push(kf);
        self.selected_index = Some(self.path.len() - 1);
    }

    /// Delete the currently selected keyframe.
    pub fn delete_selected(&mut self) {
        if let Some(i) = self.selected_index.take() {
            self.path.remove(i);
            if !self.path.is_empty() {
                self.selected_index = Some(self.path.len() - 1);
            }
        }
    }

    /// Re-sort the path while preserving the selected frame by value.
    pub fn sort(&mut self) {
        let selected_kf = self.selected_index.and_then(|i| self.path.get(i).cloned());
        self.path.sort();
        if let Some(kf) = selected_kf {
            self.selected_index = self.path.frames().iter().position(|x| *x == kf);
        }
    }

    /// Move selection to `index` and return the keyframe's time (for playhead sync).
    /// Returns `None` if the index is out of bounds.
    pub fn select(&mut self, index: usize) -> Option<f32> {
        if index < self.path.len() {
            self.selected_index = Some(index);
            let time = self.path.frames()[index].time;
            self.scroll_to(time);
            Some(time)
        } else {
            None
        }
    }

    /// Back to a pristine timeline: no path, default length and zoom.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// How much of the timeline is on screen, as a percentage: 100% is all of
    /// it, 50% is half, and smaller means zoomed further in.
    pub fn zoom_percent(&self) -> f32 {
        if self.total_duration > 0.0 {
            (self.visible_duration / self.total_duration) * 100.0
        } else {
            100.0
        }
    }

    /// Tightest zoom available — the strip showing `MIN_VISIBLE_DURATION`.
    pub fn min_zoom_percent(&self) -> f32 {
        ((MIN_VISIBLE_DURATION / self.total_duration) * 100.0).min(100.0)
    }

    /// Set the zoom by percentage, holding `anchor` (in seconds) still on screen.
    pub fn set_zoom_percent(&mut self, percent: f32, anchor: f32) {
        let percent = percent.clamp(self.min_zoom_percent(), 100.0);
        self.set_visible_duration(self.total_duration * (percent / 100.0), anchor);
    }

    /// The time at the centre of the current viewport — the natural anchor when
    /// zooming from a control rather than from the strip.
    pub fn viewport_centre(&self) -> f32 {
        self.scroll_offset + self.visible_duration / 2.0
    }

    /// Discard the authored path and re-base the timeline on a newly loaded
    /// replay. Destructive by design: the replay is the content, so keyframes
    /// authored against a different one do not carry over.
    pub fn reset_for_replay(&mut self, duration: f32) {
        self.path.clear();
        self.selected_index = None;
        self.dragging_index = None;
        self.scroll_offset = 0.0;
        self.visible_duration = duration.max(MIN_VISIBLE_DURATION);
        self.set_total_duration(duration);
    }

    /// Set the timeline's length, keeping the zoom window and scroll inside it.
    pub fn set_total_duration(&mut self, duration: f32) {
        self.total_duration = duration.max(MIN_VISIBLE_DURATION);
        self.visible_duration = self
            .visible_duration
            .clamp(MIN_VISIBLE_DURATION, self.total_duration);
        self.clamp_scroll();
    }

    /// Resize the zoom window, holding `anchor` (in seconds) still on screen.
    /// `anchor` is normally the time under the cursor, so zooming keeps the bit
    /// you are pointing at where it is.
    pub fn set_visible_duration(&mut self, visible: f32, anchor: f32) {
        let old = self.visible_duration;
        let new = visible.clamp(MIN_VISIBLE_DURATION, self.total_duration);
        if (new - old).abs() < f32::EPSILON {
            return;
        }
        // Fraction of the way across the viewport the anchor sits at.
        let frac = if old > 0.0 {
            ((anchor - self.scroll_offset) / old).clamp(0.0, 1.0)
        } else {
            0.5
        };
        self.visible_duration = new;
        self.scroll_offset = anchor - frac * new;
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let max = (self.total_duration - self.visible_duration).max(0.0);
        self.scroll_offset = self.scroll_offset.clamp(0.0, max);
    }

    /// Scroll the viewport so `time` is visible (centred if outside).
    pub fn scroll_to(&mut self, time: f32) {
        let visible_start = self.scroll_offset;
        let visible_end = visible_start + self.visible_duration;
        if time < visible_start || time > visible_end {
            let max = (self.total_duration - self.visible_duration).max(0.0);
            self.scroll_offset = (time - self.visible_duration / 2.0).clamp(0.0, max);
        }
    }

    pub fn format_time_label(&self, seconds: f32, show_decimals: bool) -> String {
        if seconds < 0.0 {
            return "0s".to_string();
        }
        let minutes = (seconds / 60.0).floor() as u32;
        let remaining_seconds = seconds % 60.0;
        if minutes > 0 {
            if show_decimals {
                format!("{}m{:05.2}s", minutes, remaining_seconds)
            } else {
                format!("{}m{:02.0}s", minutes, remaining_seconds.floor())
            }
        } else if show_decimals {
            format!("{:.2}s", remaining_seconds)
        } else {
            format!("{}s", remaining_seconds.floor())
        }
    }
}

// PlaybackController

/// Owns playback time, play/pause state, and playback mode.
pub struct PlaybackController {
    pub time: f32,
    pub playing: bool,
    pub mode_active: bool,
    pub speed: f32,
    pub last_update: Option<Instant>,
}

impl Default for PlaybackController {
    fn default() -> Self {
        Self {
            time: 0.0,
            playing: false,
            mode_active: false,
            speed: 1.0,
            last_update: None,
        }
    }
}

impl PlaybackController {
    /// Advance `time` by wall-clock delta scaled by `speed` if currently playing.
    pub fn advance(&mut self) {
        if !self.playing {
            return;
        }
        let now = Instant::now();
        if let Some(last) = self.last_update {
            // TODO: when game timescale hook is implemented, drive this from the game's
            // own delta time instead of wall-clock so playback stays in sync automatically.
            self.time += (now - last).as_secs_f32() * self.speed;
        }
        self.last_update = Some(now);
    }

    /// Toggle play/pause. `end_time` is the total content duration; rewinds to 0
    /// when the playhead is at or past it.
    pub fn toggle(&mut self, end_time: f32) {
        self.playing = !self.playing;
        if self.playing {
            self.last_update = Some(Instant::now());
            self.mode_active = true;
            if self.time >= end_time {
                self.time = 0.0;
            }
        }
    }
}

/// Tracks recording state and drives the capture pipeline in `game`.
pub struct RecordingSession {
    pub active: bool,
    /// The name typed by the user (doubles as the active recording name).
    pub name: String,
}

impl Default for RecordingSession {
    fn default() -> Self {
        Self {
            active: false,
            name: String::new(),
        }
    }
}

impl RecordingSession {
    /// Create the replay file and hand it to the capture pipeline.
    pub fn start(&mut self) -> anyhow::Result<()> {
        use crate::replay::Replay;
        let mut replay = Replay::new(&self.name);
        replay.create_file()?;
        capture::start(replay);
        self.active = true;
        Ok(())
    }

    /// Finish the replay file and stop capturing.
    pub fn stop(&mut self) {
        if let Some(mut replay) = capture::stop() {
            if let Err(e) = replay.finish() {
                log::error!("failed to finish replay: {e}");
            }
        }
        self.active = false;
        self.name.clear();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraMode {
    /// You fly the camera, and the authored keyframe path moves it during
    /// playback. The camera the replay was shot with is ignored.
    #[default]
    Freecam,
    /// Play back through the camera the replay was recorded with. The authored
    /// path is ignored.
    Pov,
    /// Drag to circle a pivot, which may be a fixed point or the replay's
    /// skater. Keys placed here store the orbit rather than a world pose, so a
    /// run of them sweeps round the pivot at a constant radius. Paused, the
    /// mouse owns the camera, playing, the path does.
    Orbit,
}

impl CameraMode {
    const ALL: [CameraMode; 3] = [CameraMode::Freecam, CameraMode::Pov, CameraMode::Orbit];

    fn label(self) -> &'static str {
        match self {
            CameraMode::Freecam => "Freecam",
            CameraMode::Pov => "POV",
            CameraMode::Orbit => "Orbit",
        }
    }
}

/// Where the camera aims when following the skater in a replay frame. The
/// recorded position is the root bone, so `aim_height` lifts it off the feet.
fn skater_target(frame: Option<&ReplayState>, aim_height: f32) -> Option<Vec3> {
    let pos = frame?.skaters.first()?.world_pos;
    Some(Vec3::new(pos[0], pos[1] + aim_height, pos[2]))
}

pub struct UiSystem {
    pub is_showing: bool,
    pub timeline: Timeline,
    pub playback: PlaybackController,
    pub recording: RecordingSession,
    pub loaded_replay: Option<LoadedReplay>,
    pub load_name: String,
    pub camera_mode: CameraMode,
    pub orbit: OrbitCamera,
    pub aim_height: f32,
    last_logged_frame: usize,
}

impl Default for UiSystem {
    fn default() -> Self {
        Self {
            is_showing: true,
            timeline: Timeline::default(),
            playback: PlaybackController::default(),
            recording: RecordingSession::default(),
            loaded_replay: None,
            load_name: String::new(),
            camera_mode: CameraMode::default(),
            orbit: OrbitCamera {
                target: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.3,
                distance: DEFAULT_ORBIT_DISTANCE,
            },
            aim_height: 40.0,
            last_logged_frame: usize::MAX,
        }
    }
}

impl UiSystem {
    pub fn is_showing(&self) -> bool {
        self.is_showing && game::in_freecam()
    }

    pub fn toggle_showing(&mut self) {
        if !game::in_freecam() {
            return;
        }
        self.is_showing = !self.is_showing;
        unsafe { ShowCursor(self.is_showing) };
    }

    /// Per-frame update: advance time, apply the interpolated camera pose, and
    /// return the replay frame to draw this tick, if any.
    pub fn update(&mut self) -> Option<ReplayState> {
        self.playback.advance();

        let end_time = self.playback_end_time();

        // Replay playback
        let frame = if self.playback.mode_active {
            self.loaded_replay.as_ref().and_then(|r| {
                let t = self.playback.time * 60.0;
                let frame_idx = (t as usize).min(r.frames.len().saturating_sub(1));
                if frame_idx != self.last_logged_frame {
                    let player_pos = r
                        .frames
                        .get(frame_idx)
                        .and_then(|f| f.skaters.first())
                        .map(|s| {
                            format!(
                                "({:.1},{:.1},{:.1})",
                                s.world_pos[0], s.world_pos[1], s.world_pos[2]
                            )
                        })
                        .unwrap_or_else(|| "?".into());
                    log::debug!(
                        "[playback] frame={}/{} t={:.2}s player={}",
                        frame_idx,
                        r.frames.len(),
                        self.playback.time,
                        player_pos
                    );
                    self.last_logged_frame = frame_idx;
                }
                r.sample(t)
            })
        } else {
            self.last_logged_frame = usize::MAX;
            None
        };

        // Camera
        // Recording is not handled here, the capture pass samples the camera
        // itself, so the replay frame stays self-contained.
        //
        // The two sources are mutually exclusive on purpose. Both write the same
        // camera object every frame, so running them together means whichever
        // wrote last wins and neither looks right.
        if !capture::is_recording() {
            let skater = skater_target(frame.as_ref(), self.aim_height);
            let pose: Option<CameraPose> = match self.camera_mode {
                // `frame` is only Some in playback mode, which gates this.
                CameraMode::Pov => frame.as_ref().and_then(|f| f.camera),
                CameraMode::Freecam => {
                    if self.playback.mode_active || self.playback.playing {
                        self.timeline.path.sample(self.playback.time, skater)
                    } else {
                        None
                    }
                }
                // Playing runs the path, orbit keys included; paused hands the
                // camera back to the mouse so keys can be placed.
                CameraMode::Orbit => self
                    .playback
                    .playing
                    .then(|| self.timeline.path.sample(self.playback.time, skater))
                    .flatten()
                    .or_else(|| {
                        let fov = game::camera().map_or(DEFAULT_FOV, |c| c.fov);
                        Some(self.orbit.pose_around(self.orbit.pivot(skater), fov))
                    }),
            };
            if let Some(pose) = pose {
                game::set_camera(&pose);
            }
        }

        // Stop playing when we reach the end of all content.
        if self.playback.playing && end_time > 0.0 && self.playback.time >= end_time {
            self.playback.playing = false;
            log::debug!("[playback] reached end ({:.2}s), stopping", end_time);
        }

        frame
    }

    /// The time at which all playback content ends — max of replay duration and last keyframe.
    fn playback_end_time(&self) -> f32 {
        let replay_end = self
            .loaded_replay
            .as_ref()
            .map(|r| r.frame_count as f32 / 60.0)
            .unwrap_or(0.0);
        let path_end = self
            .timeline
            .path
            .frames()
            .last()
            .map(|f| f.time)
            .unwrap_or(0.0);
        replay_end.max(path_end)
    }

    /// Pivot the orbit around whatever the camera is looking at, so entering
    /// orbit mode does not jump the view.
    fn recentre_orbit(&mut self) {
        if let Some(pose) = game::camera() {
            self.orbit =
                OrbitCamera::from_pose(&pose, DEFAULT_ORBIT_DISTANCE).with_yaw_near(self.orbit.yaw);
        }
    }

    /// What a keyframe placed now should remember of the orbit, if anything.
    fn orbit_for_new_keyframe(&self) -> Option<OrbitCamera> {
        (self.camera_mode == CameraMode::Orbit).then_some(self.orbit)
    }

    /// Picking an orbit key while orbiting puts the mouse back where that key
    /// was, so it can be adjusted and overwritten.
    fn load_selected_orbit(&mut self) {
        if self.camera_mode != CameraMode::Orbit {
            return;
        }
        let selected = self
            .timeline
            .selected_index
            .and_then(|i| self.timeline.path.get(i));
        if let Some(orbit) = selected.and_then(|kf| kf.orbit) {
            self.orbit = orbit;
        }
    }

    /// Drag orbits, shift+drag pans the pivot, wheel zooms. Only over the game
    /// view: input aimed at the timeline or a window stays with egui.
    pub fn update_orbit_input(&mut self, ctx: &eguiContext) {
        // Playing, the path owns the camera; dragging then would only bank up
        // a jump for the moment it pauses.
        if self.camera_mode != CameraMode::Orbit
            || self.playback.playing
            || ctx.is_pointer_over_area()
            || ctx.is_using_pointer()
        {
            return;
        }

        let (dragging, shift, delta, scroll) = ctx.input(|i| {
            (
                i.pointer.primary_down() || i.pointer.secondary_down(),
                i.modifiers.shift,
                i.pointer.delta(),
                i.smooth_scroll_delta.y,
            )
        });

        if dragging && shift {
            let speed = self.orbit.distance * ORBIT_PAN_SPEED;
            self.orbit.pan(-delta.x * speed, delta.y * speed);
        } else if dragging {
            self.orbit
                .rotate(-delta.x * ORBIT_ROTATE_SPEED, delta.y * ORBIT_ROTATE_SPEED);
        }
        if scroll != 0.0 {
            self.orbit.zoom((-scroll * ORBIT_ZOOM_SPEED).exp());
        }
    }

    // egui panels

    pub fn update_replay_detail(&mut self, ctx: &eguiContext) {
        let top_padding = 880.0;
        let right_padding = 40.0;
        let window_width = 220.0;

        let screen_rect = ctx.screen_rect();
        let window_pos = egui::pos2(
            screen_rect.right() - window_width - right_padding,
            top_padding,
        );

        egui::Window::new("Replay")
            .default_pos(window_pos)
            .default_width(window_width)
            .collapsible(true)
            .resizable(false)
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.heading("Replay");
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.recording.name)
                                .desired_width(120.0),
                        );

                        let button_txt = if self.recording.active {
                            "⏹ Stop Recording"
                        } else {
                            "🔴 Record"
                        };
                        let can_record = self.recording.active || self.loaded_replay.is_none();

                        if ui
                            .add_enabled(can_record, egui::Button::new(button_txt))
                            .clicked()
                        {
                            if self.recording.active {
                                self.recording.stop();
                            } else if !self.recording.name.is_empty() {
                                match self.recording.start() {
                                    Ok(()) => {
                                        log::debug!("started recording: {}", self.recording.name)
                                    }
                                    Err(e) => log::error!("failed creating replay: {}", e),
                                }
                            }
                        }
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Load:");
                        ui.add_enabled_ui(self.loaded_replay.is_none(), |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.load_name)
                                    .desired_width(120.0),
                            );
                        });

                        let can_load = !self.load_name.is_empty() && self.loaded_replay.is_none();
                        if ui
                            .add_enabled(can_load, egui::Button::new("📂 Load"))
                            .clicked()
                        {
                            match LoadedReplay::load(&self.load_name) {
                                Ok(r) => {
                                    log::info!(
                                        "loaded replay: {} ({} frames)",
                                        r.name,
                                        r.frame_count
                                    );
                                    // The replay is the content: it defines the
                                    // timeline's length and clears any path authored
                                    // against a previous one.
                                    let replay_duration = r.frame_count as f32 / 60.0;
                                    let discarded = self.timeline.path.len();
                                    self.timeline.reset_for_replay(replay_duration);
                                    if discarded > 0 {
                                        log::info!(
                                            "discarded {discarded} keyframe(s) on replay load"
                                        );
                                    }
                                    self.playback.time = 0.0;
                                    self.playback.playing = false;
                                    self.last_logged_frame = usize::MAX;
                                    self.loaded_replay = Some(r);
                                }
                                Err(e) => log::error!("failed to load replay: {e}"),
                            }
                        }

                        if self.loaded_replay.is_some() && ui.button("⏏ Unload").clicked() {
                            // Unloading the content resets the timeline with it.
                            self.timeline.reset();
                            self.playback.time = 0.0;
                            self.playback.playing = false;
                            self.last_logged_frame = usize::MAX;
                            self.loaded_replay = None;
                            self.load_name.clear();
                        }
                    });

                    if let Some(r) = &self.loaded_replay {
                        ui.label(format!("▶ {} ({} frames)", r.name, r.frame_count));
                    }
                });
            });
    }

    pub fn update_keyframe_detail(&mut self, ctx: &eguiContext) {
        let top_padding = 480.0;
        let right_padding = 40.0;
        let window_width = 220.0;

        let screen_rect = ctx.screen_rect();
        let window_pos = egui::pos2(
            screen_rect.right() - window_width - right_padding,
            top_padding,
        );

        egui::Window::new("Keyframe")
            .default_pos(window_pos)
            .default_width(window_width)
            .collapsible(true)
            .resizable(false)
            .show(ctx, |ui| {
                let orbiting = self.camera_mode == CameraMode::Orbit;
                if let Some(i) = self.timeline.selected_index {
                    let Some(pre_edit) = self.timeline.path.get(i).cloned() else {
                        return;
                    };
                    let mut kf = pre_edit.clone();
                    let live_orbit = self.orbit_for_new_keyframe();

                    ui.vertical(|ui| {
                        ui.heading("Keyframe details");
                        ui.horizontal(|ui| {
                            ui.label("Time");
                            ui.add(DragValue::new(&mut kf.time).speed(0.01).suffix("s"));
                        });
                        // An orbit key's position comes from its orbit, so the
                        // snapshot is not worth editing.
                        ui.add_enabled_ui(kf.orbit.is_none(), |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Position");
                                ui.add(
                                    DragValue::new(&mut kf.pose.position.x)
                                        .speed(0.4)
                                        .prefix("x: "),
                                );
                                ui.add(
                                    DragValue::new(&mut kf.pose.position.y)
                                        .speed(0.4)
                                        .prefix("y: "),
                                );
                                ui.add(
                                    DragValue::new(&mut kf.pose.position.z)
                                        .speed(0.4)
                                        .prefix("z: "),
                                );
                            })
                        });
                        ui.horizontal(|ui| {
                            ui.label("Rotation");
                            ui.add_enabled_ui(false, |ui| {
                                ui.add(
                                    DragValue::new(&mut kf.pose.orientation.0)
                                        .speed(0.01)
                                        .prefix("x: "),
                                );
                                ui.add(
                                    DragValue::new(&mut kf.pose.orientation.1)
                                        .speed(0.01)
                                        .prefix("y: "),
                                );
                                ui.add(
                                    DragValue::new(&mut kf.pose.orientation.2)
                                        .speed(0.01)
                                        .prefix("z: "),
                                );
                                ui.add(
                                    DragValue::new(&mut kf.pose.orientation.3)
                                        .speed(0.01)
                                        .prefix("w: "),
                                );
                            });
                        });
                        ui.horizontal(|ui| {
                            ui.label("FOV");
                            ui.add(
                                DragValue::new(&mut kf.pose.fov)
                                    .speed(1.0)
                                    .range(15.0..=140.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Path tension");
                            ui.add(DragValue::new(&mut kf.tension).speed(0.01).range(0.0..=1.0));
                        });
                        // Only while orbiting: these are the values the mouse
                        // is flying, so out of orbit mode they are neither
                        // editable nor worth looking at.
                        if let Some(orbit) = kf.orbit.as_mut().filter(|_| orbiting) {
                            ui.separator();
                            ui.label("Orbit");
                            ui.horizontal(|ui| {
                                ui.label("Distance");
                                ui.add(
                                    DragValue::new(&mut orbit.distance)
                                        .speed(1.0)
                                        .range(OrbitCamera::MIN_DISTANCE..=f32::MAX),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label("Yaw");
                                ui.drag_angle(&mut orbit.yaw);
                                ui.label("Pitch");
                                ui.drag_angle(&mut orbit.pitch);
                            });
                            // Re-clamp whatever the drag fields let through.
                            orbit.rotate(0.0, 0.0);
                        }

                        // Captures what drives the camera now: orbiting makes
                        // this an orbit key, anything else a plain one.
                        if ui.button("Overwrite with current camera").clicked() {
                            if let Some(pose) = game::camera() {
                                kf.pose = pose;
                                kf.orbit = live_orbit;
                            }
                        }

                        ui.separator();
                    });

                    if pre_edit != kf {
                        let was_on_keyframe = (self.playback.time - pre_edit.time).abs() < 1e-3;

                        if let Some(frame) = self.timeline.path.get_mut(i) {
                            *frame = kf.clone();
                        }
                        if pre_edit.time != kf.time {
                            self.timeline.sort();
                        }
                        self.load_selected_orbit();

                        // Live preview: snap playhead and push new pose immediately.
                        if self.playback.mode_active && was_on_keyframe {
                            self.playback.time = kf.time;
                            self.playback.last_update = Some(Instant::now());
                            game::set_camera(&kf.resolve(None));
                        }
                    }
                } else {
                    ui.label("No keyframe selected.");
                }

                // The pivot and the mouse bindings belong to the orbit rig
                // rather than to any one key, so they stay reachable with
                // nothing selected - you need them to frame the first key.
                if orbiting {
                    ui.separator();
                    ui.label("Orbit pivot");
                    ui.horizontal(|ui| {
                        ui.label("Aim height");
                        ui.add(
                            DragValue::new(&mut self.aim_height)
                                .speed(0.5)
                                .range(0.0..=f32::MAX),
                        );
                    })
                    .response
                    .on_hover_text(
                        "How far above the skater's feet the pivot sits. The recorded \
                         position is the root bone, which is between them.",
                    );
                    if ui
                        .button("🎯 Re-centre on camera")
                        .on_hover_text(
                            "Pivot on whatever the camera is looking at now. Only bites \
                             with no replay skater to orbit, as does panning.",
                        )
                        .clicked()
                    {
                        self.recentre_orbit();
                    }
                    ui.weak("drag orbit · shift+drag pan · wheel zoom");
                }
            });
    }

    pub fn update_timeline(&mut self, ctx: &eguiContext) {
        let prev_selected = self.timeline.selected_index;
        self.draw_timeline(ctx);
        if self.timeline.selected_index != prev_selected {
            self.load_selected_orbit();
        }
    }

    fn draw_timeline(&mut self, ctx: &eguiContext) {
        TopBottomPanel::bottom(egui::Id::new("TimelinePanel")).show(ctx, |ui| {
            // Two bars: what you are editing, then how you are looking at it.
            ui.horizontal(|ui| {
                ui.label("Keyframe");
                if ui.button("➕ Add").clicked() {
                    if let Some(pose) = game::camera() {
                        let orbit = self.orbit_for_new_keyframe();
                        self.timeline.add_keyframe(Keyframe {
                            time: self.playback.time,
                            pose,
                            tension: 0.25,
                            orbit,
                        });
                    }
                }

                ui.add_enabled_ui(self.timeline.selected_index.is_some(), |ui| {
                    if ui.button("📋 Duplicate").clicked() {
                        if let Some(i) = self.timeline.selected_index {
                            if let Some(kf) = self.timeline.path.get(i).cloned() {
                                let mut dup = kf;
                                dup.time += 1.0;
                                self.timeline.add_keyframe(dup);
                            }
                        }
                    }
                });

                ui.add_enabled_ui(self.timeline.selected_index.is_some(), |ui| {
                    if ui.button("❌ Delete").clicked() {
                        self.timeline.delete_selected();
                    }
                });

                ui.separator();

                if ui
                    .checkbox(&mut self.playback.mode_active, "Playback mode")
                    .changed()
                    && !self.playback.mode_active
                {
                    self.playback.playing = false;
                }

                let can_go_prev = self.timeline.selected_index.is_some_and(|i| i > 0);
                if ui
                    .add_enabled(can_go_prev, egui::Button::new("⏪ Previous"))
                    .clicked()
                {
                    if let Some(i) = self.timeline.selected_index {
                        if let Some(time) = self.timeline.select(i - 1) {
                            self.playback.time = time;
                            self.playback.last_update = Some(Instant::now());
                        }
                    }
                }

                let play_pause_icon = if self.playback.playing {
                    "⏸ Pause"
                } else {
                    "▶ Play"
                };
                if ui.button(play_pause_icon).clicked() {
                    self.playback.toggle(self.playback_end_time());
                }

                let can_go_next = self
                    .timeline
                    .selected_index
                    .is_some_and(|i| i < self.timeline.path.len() - 1);
                if ui
                    .add_enabled(can_go_next, egui::Button::new("⏩ Next"))
                    .clicked()
                {
                    if let Some(i) = self.timeline.selected_index {
                        if let Some(time) = self.timeline.select(i + 1) {
                            self.playback.time = time;
                            self.playback.last_update = Some(Instant::now());
                        }
                    }
                }

                ui.separator();
                ui.label("Speed");
                ui.add(
                    DragValue::new(&mut self.playback.speed)
                        .speed(0.01)
                        .range(0.01..=4.0)
                        .suffix("x"),
                );
            });

            ui.horizontal(|ui| {
                let prev_mode = self.camera_mode;
                egui::ComboBox::from_id_salt("CameraMode")
                    .selected_text(self.camera_mode.label())
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        for mode in CameraMode::ALL {
                            ui.selectable_value(&mut self.camera_mode, mode, mode.label());
                        }
                    });
                if self.camera_mode == CameraMode::Orbit && prev_mode != CameraMode::Orbit {
                    // Pick the rig up around whatever the camera already looks
                    // at, or where the selected orbit key left it, so entering
                    // the mode does not jump the view.
                    self.recentre_orbit();
                    self.load_selected_orbit();
                }

                ui.separator();

                ui.label("Duration");
                let mut total = self.timeline.total_duration;
                if ui
                    .add(
                        DragValue::new(&mut total)
                            .speed(1.0)
                            .range(MIN_VISIBLE_DURATION..=f32::MAX)
                            .suffix("s"),
                    )
                    .changed()
                {
                    self.timeline.set_total_duration(total);
                }

                ui.separator();

                ui.label("Zoom");
                let mut zoom = self.timeline.zoom_percent();
                if ui
                    .add(
                        DragValue::new(&mut zoom)
                            .speed(0.5)
                            .range(self.timeline.min_zoom_percent()..=100.0)
                            .fixed_decimals(1)
                            .suffix("%"),
                    )
                    .changed()
                {
                    let centre = self.timeline.viewport_centre();
                    self.timeline.set_zoom_percent(zoom, centre);
                }
                if ui.button("Fit").clicked() {
                    self.timeline.set_zoom_percent(100.0, 0.0);
                }

                ui.separator();

                // Takes the rest of the bar: a scrollbar you can actually aim at.
                ui.label("Scroll");
                let max_scroll =
                    (self.timeline.total_duration - self.timeline.visible_duration).max(0.0);
                ui.add_enabled(
                    max_scroll > 0.0,
                    egui::Slider::new(&mut self.timeline.scroll_offset, 0.0..=max_scroll)
                        .show_value(false),
                );
            });

            let (space_pressed, delete_pressed) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Space),
                    i.key_pressed(egui::Key::Delete),
                )
            });
            if space_pressed {
                self.playback.toggle(self.playback_end_time());
            }
            if delete_pressed && self.timeline.selected_index.is_some() {
                self.timeline.delete_selected();
            }

            let pixels_per_second = ui.available_width() / self.timeline.visible_duration;
            let timeline_height = 50.0;
            let (timeline_rect, timeline_response) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), timeline_height),
                Sense::click_and_drag(),
            );
            let painter = ui.painter_at(timeline_rect);
            let timeline_top = timeline_rect.top();
            let timeline_left = timeline_rect.left();
            painter.rect_filled(timeline_rect, 0.0, Color32::DARK_GRAY);

            let visible_start = self.timeline.scroll_offset;
            let visible_end = visible_start + self.timeline.visible_duration;
            let start_second = visible_start.floor() as i32;
            let end_second = visible_end.ceil() as i32;
            let time_to_x = |t: f32| timeline_left + (t - visible_start) * pixels_per_second;

            for second_i32 in start_second..=end_second {
                let second = second_i32 as f32;
                if second < 0.0 {
                    continue;
                }
                let x = time_to_x(second);
                if x >= timeline_left && x <= timeline_rect.right() {
                    painter.line_segment(
                        [
                            Pos2::new(x, timeline_top),
                            Pos2::new(x, timeline_top + 10.0),
                        ],
                        (1.0, Color32::WHITE),
                    );
                    let time_text = self.timeline.format_time_label(second, false);
                    painter.text(
                        Pos2::new(x + 2.0, timeline_top + 12.0),
                        egui::Align2::LEFT_TOP,
                        time_text,
                        egui::FontId::monospace(10.0),
                        Color32::WHITE,
                    );
                }
            }

            let mut needs_sort = false;
            for i in 0..self.timeline.path.len() {
                let kf_time = {
                    let frames = self.timeline.path.frames();
                    let t = frames[i].time;
                    if t < visible_start || t > visible_end {
                        continue;
                    }
                    t
                };

                let x = time_to_x(kf_time);
                let kf_top = timeline_top + 24.0;
                let kf_bottom = timeline_rect.bottom() - 3.0;
                let color = if self.timeline.selected_index == Some(i) {
                    Color32::YELLOW
                } else if self.timeline.path.frames()[i].orbit.is_some() {
                    Color32::from_rgb(255, 160, 60)
                } else {
                    Color32::LIGHT_BLUE
                };
                let line_rect =
                    Rect::from_min_max(Pos2::new(x - 3.0, kf_top), Pos2::new(x + 3.0, kf_bottom));
                let response = ui.allocate_rect(line_rect, Sense::click_and_drag());
                painter.rect_filled(line_rect, 4.0, color);

                if response.clicked() {
                    self.timeline.selected_index = Some(i);
                    if self.playback.mode_active {
                        self.playback.time = kf_time;
                        self.playback.last_update = Some(Instant::now());
                    }
                }
                if response.drag_started() {
                    self.timeline.dragging_index = Some(i);
                    self.timeline.selected_index = Some(i);
                }
                if response.dragged() && self.timeline.dragging_index == Some(i) {
                    if let Some(kf) = self.timeline.path.get_mut(i) {
                        kf.time += response.drag_delta().x / pixels_per_second;
                        kf.time = kf.time.clamp(0.0, self.timeline.total_duration);
                    }
                }
                if response.drag_stopped() {
                    needs_sort = true;
                }
            }
            if needs_sort {
                self.timeline.sort();
            }

            // Draw playhead.
            let pb_time = self.playback.time;
            if pb_time >= visible_start && pb_time <= visible_end {
                let playhead_x = time_to_x(pb_time);
                painter.line_segment(
                    [
                        Pos2::new(playhead_x, timeline_rect.top()),
                        Pos2::new(playhead_x, timeline_rect.bottom()),
                    ],
                    (2.0, Color32::GREEN),
                );
                let time_text = self.timeline.format_time_label(pb_time, true);
                painter.text(
                    Pos2::new(playhead_x + 4.0, timeline_rect.top()),
                    egui::Align2::LEFT_TOP,
                    time_text,
                    egui::FontId::monospace(12.0),
                    Color32::GREEN,
                );
            }

            // Wheel over the strip zooms around the time under the cursor.
            if timeline_response.hovered() {
                let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll_y != 0.0 {
                    let anchor = ui
                        .input(|i| i.pointer.hover_pos())
                        .map(|p| ((p.x - timeline_left) / pixels_per_second) + visible_start)
                        .unwrap_or(visible_start + self.timeline.visible_duration / 2.0);
                    // Wheel up zooms in (less of the timeline on screen), down out.
                    let step = if scroll_y > 0.0 { 1.0 / 1.15 } else { 1.15 };
                    let zoom = self.timeline.zoom_percent() * step;
                    self.timeline.set_zoom_percent(zoom, anchor);
                }
            }

            if timeline_response.clicked() || timeline_response.dragged() {
                if let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos()) {
                    let relative_x = pointer_pos.x - timeline_rect.left();
                    let new_time = (relative_x / pixels_per_second) + visible_start;
                    self.playback.time = new_time.clamp(0.0, self.timeline.total_duration);
                    self.playback.last_update = Some(Instant::now());
                }
            }
            if timeline_response.dragged() {
                self.playback.playing = false;
            }
        });
    }
}

// Render loop (called by egui-d3d9 every Present)

pub fn ui_render_loop(ctx: &eguiContext, _i: &mut i32) {
    let mut ui = UI_SYSTEM
        .get_or_init(|| Mutex::new(UiSystem::default()))
        .lock()
        .unwrap();

    if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
        ui.toggle_showing();
    }

    if ui.playback.playing || ui.recording.active {
        ctx.request_repaint();
    }

    // F2 starts/stops recording regardless of freecam — gameplay happens outside freecam.
    if ctx.input(|i| i.key_pressed(egui::Key::F2)) {
        if ui.recording.active {
            ui.recording.stop();
        } else if !ui.recording.name.is_empty() {
            match ui.recording.start() {
                Ok(()) => log::debug!("started recording via F2: {}", ui.recording.name),
                Err(e) => log::error!("failed creating replay: {}", e),
            }
        }
    }

    if ui.recording.active {
        ctx.debug_painter().text(
            Pos2::new(0., 40.),
            Align2::LEFT_TOP,
            format!("Recording: {}", ui.recording.name),
            FontId::default(),
            Color32::LIGHT_RED,
        );
        ctx.debug_painter().text(
            Pos2::new(0., 60.),
            Align2::LEFT_TOP,
            "Press F2 to stop recording.",
            FontId::default(),
            Color32::WHITE,
        );
    }

    if !ui.is_showing() {
        return;
    }

    // No camera yet means freecam has never been entered, so there is nothing
    // to author against.
    if game::camera().is_some() {
        ctx.debug_painter().text(
            Pos2::new(0., 0.),
            Align2::LEFT_TOP,
            version_string(),
            FontId::default(),
            Color32::LIGHT_YELLOW,
        );
        ui.update_timeline(ctx);
        ui.update_replay_detail(ctx);
        ui.update_keyframe_detail(ctx);
        ui.update_orbit_input(ctx);
    }
}
