use std::{
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

use anyhow::{Context, Result};

use crate::{
    camera::CameraPose,
    math::{lerp_arr, Quat, Vec3},
};

const FLK_REPLAY_VERSION: u8 = 2;
const REPLAY_FOLDER: &str = "Replays";

pub struct Replay {
    file_path: PathBuf,
    writer: Option<BufWriter<std::fs::File>>,
    frame_count: u64,
}

/// One skater's pose for a single frame.
#[derive(Debug, Clone)]
pub struct SkaterState {
    pub world_pos: [f32; 3],
    pub world_matrix: [f32; 16],
    pub bone_quats: [[f32; 4]; 50],
    pub bone_trans: [[f32; 4]; 50],
}

/// One frame of a replay.
#[derive(Debug, Clone, Default)]
pub struct ReplayState {
    pub skaters: Vec<SkaterState>,
    pub camera: Option<CameraPose>,
}

impl Replay {
    pub fn new(name: &str) -> Self {
        Self {
            file_path: PathBuf::from(REPLAY_FOLDER).join(name.to_owned() + ".flk"),
            writer: None,
            frame_count: 0,
        }
    }

    /// Creates the file, writes the header, and opens the streaming writer.
    pub fn create_file(&mut self) -> Result<()> {
        log::debug!("opening replay file: {:?}", self.file_path);
        let prefix = self.file_path.parent().unwrap();
        std::fs::create_dir_all(prefix).context("failed to create Replays directory")?;

        let file =
            std::fs::File::create(&self.file_path).context("failed to create replay file")?;
        let mut writer = BufWriter::new(file);
        writer.write_all(&[FLK_REPLAY_VERSION])?;
        // Placeholder: frame count will be written in finish().
        writer.write_all(&0u64.to_le_bytes())?;
        self.writer = Some(writer);
        Ok(())
    }

    /// Writes one captured frame to disk and advances the frame counter.
    /// No-ops if the frame holds no skaters (e.g. none loaded yet).
    pub fn write_frame(&mut self, state: &ReplayState) -> Result<()> {
        if state.skaters.is_empty() {
            return Ok(());
        }
        let writer = self.writer.as_mut().context("replay writer not open")?;

        writer.write_all(&[state.skaters.len() as u8])?;
        for skater in &state.skaters {
            write_f32s(writer, &skater.world_pos)?;
            write_f32s(writer, &skater.world_matrix)?;
            for quat in &skater.bone_quats {
                write_f32s(writer, quat)?;
            }
            for trans in &skater.bone_trans {
                write_f32s(writer, trans)?;
            }
        }
        // Camera: a presence byte, then the pose. A frame recorded before the
        // camera was found genuinely has none, so it is not worth a pose of
        // zeros that reads back as a valid (and wrong) orientation.
        match &state.camera {
            Some(cam) => {
                writer.write_all(&[1])?;
                write_f32s(writer, &cam.position.to_array())?;
                write_f32s(writer, &cam.orientation.to_array())?;
                write_f32s(writer, &[cam.fov])?;
            }
            None => writer.write_all(&[0])?,
        }
        self.frame_count += 1;
        Ok(())
    }

    /// Flushes the writer and patches the frame count into the file header.
    pub fn finish(&mut self) -> Result<()> {
        if let Some(writer) = self.writer.take() {
            let mut file = writer
                .into_inner()
                .map_err(|e| anyhow::anyhow!("flush failed: {}", e.error()))?;
            file.seek(SeekFrom::Start(1))?;
            file.write_all(&self.frame_count.to_le_bytes())?;
            log::info!(
                "replay saved: {} frames → {:?}",
                self.frame_count,
                self.file_path
            );
        }
        Ok(())
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
}

pub struct LoadedReplay {
    pub name: String,
    pub frame_count: u64,
    pub frames: Vec<ReplayState>,
}

impl LoadedReplay {
    pub fn load(name: &str) -> Result<Self> {
        let file_path = PathBuf::from(REPLAY_FOLDER).join(name.to_owned() + ".flk");
        let mut file = std::fs::File::open(&file_path)
            .with_context(|| format!("failed to open replay file: {:?}", file_path))?;

        let mut version = [0u8; 1];
        file.read_exact(&mut version)
            .context("failed to read replay version")?;
        if version[0] != FLK_REPLAY_VERSION {
            anyhow::bail!("unsupported replay version: {}", version[0]);
        }

        let mut frame_count_bytes = [0u8; 8];
        file.read_exact(&mut frame_count_bytes)
            .context("failed to read frame count")?;
        let frame_count = u64::from_le_bytes(frame_count_bytes);

        let mut frames = Vec::with_capacity(frame_count as usize);

        for _ in 0..frame_count {
            let mut skater_count = [0u8; 1];
            file.read_exact(&mut skater_count)?;
            let skater_count = skater_count[0] as usize;

            let mut skaters = Vec::with_capacity(skater_count);
            for _ in 0..skater_count {
                let world_pos = read_f32s::<3>(&mut file)?;
                let world_matrix = read_f32s::<16>(&mut file)?;

                let mut bone_quats = [[0f32; 4]; 50];
                for quat in bone_quats.iter_mut() {
                    *quat = read_f32s::<4>(&mut file)?;
                }
                let mut bone_trans = [[0f32; 4]; 50];
                for trans in bone_trans.iter_mut() {
                    *trans = read_f32s::<4>(&mut file)?;
                }

                skaters.push(SkaterState {
                    world_pos,
                    world_matrix,
                    bone_quats,
                    bone_trans,
                });
            }

            // Camera is written after all skaters, behind a presence byte.
            let mut has_camera = [0u8; 1];
            file.read_exact(&mut has_camera)?;
            let camera = if has_camera[0] != 0 {
                Some(CameraPose {
                    position: Vec3::from_array(read_f32s(&mut file)?),
                    orientation: Quat::from_array(read_f32s(&mut file)?),
                    fov: read_f32s::<1>(&mut file)?[0],
                })
            } else {
                None
            };

            frames.push(ReplayState { skaters, camera });
        }

        Ok(Self {
            name: name.to_owned(),
            frame_count,
            frames,
        })
    }

    pub fn get_frame(&self, index: usize) -> Option<&ReplayState> {
        self.frames.get(index)
    }

    /// Sample the replay at a fractional frame index, interpolating between the
    /// two surrounding frames. `t` is in frame units (seconds * 60).
    pub fn sample(&self, t: f32) -> Option<ReplayState> {
        if self.frames.is_empty() {
            return None;
        }
        let max = self.frames.len() - 1;
        let t = t.clamp(0.0, max as f32);
        let i0 = (t as usize).min(max);
        let i1 = (i0 + 1).min(max);
        let frac = t - i0 as f32;

        if i0 == i1 {
            return Some(self.frames[i0].clone());
        }

        let a = &self.frames[i0];
        let b = &self.frames[i1];

        // Skaters pair up by position; a frame with fewer is the limit.
        let skaters = a
            .skaters
            .iter()
            .zip(b.skaters.iter())
            .map(|(sa, sb)| {
                let mut bone_quats = [[0f32; 4]; 50];
                for i in 0..50 {
                    bone_quats[i] = slerp_quat(sa.bone_quats[i], sb.bone_quats[i], frac);
                }
                let mut bone_trans = [[0f32; 4]; 50];
                for i in 0..50 {
                    bone_trans[i] = lerp_arr(sa.bone_trans[i], sb.bone_trans[i], frac);
                }
                SkaterState {
                    world_pos: lerp_arr(sa.world_pos, sb.world_pos, frac),
                    world_matrix: lerp_arr(sa.world_matrix, sb.world_matrix, frac),
                    bone_quats,
                    bone_trans,
                }
            })
            .collect();

        let camera = match (&a.camera, &b.camera) {
            (Some(ca), Some(cb)) => Some(CameraPose::lerp(ca, cb, frac)),
            (camera, _) => *camera,
        };

        Some(ReplayState { skaters, camera })
    }
}

fn slerp_quat(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    // quat_slerp already takes the shortest path.
    let r = nalgebra_glm::quat_slerp(&Quat::from_array(a).into(), &Quat::from_array(b).into(), t);
    Quat::from(r).to_array()
}

fn write_f32s(w: &mut impl Write, values: &[f32]) -> Result<()> {
    for v in values {
        w.write_all(&v.to_le_bytes())?;
    }
    Ok(())
}

fn read_f32s<const N: usize>(r: &mut impl Read) -> Result<[f32; N]> {
    let mut out = [0f32; N];
    for v in out.iter_mut() {
        let mut buf = [0u8; 4];
        r.read_exact(&mut buf)?;
        *v = f32::from_le_bytes(buf);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_replay() {
        let replay = Replay::new("alcatraz");
        assert_eq!(replay.frame_count(), 0);
    }

    #[test]
    fn write_frame_without_skaters_is_a_noop() {
        let mut replay = Replay::new("alcatraz");
        // No writer open: an empty frame must bail out before touching it.
        replay.write_frame(&ReplayState::default()).unwrap();
        assert_eq!(replay.frame_count(), 0);
    }
}
