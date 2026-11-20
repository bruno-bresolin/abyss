use crate::{
    camera::{Camera, Location},
    engine::{Engine, Frame, Settings},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::BufWriter,
    path::Path,
};

pub fn save_png(frame: &Frame, path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "cannot create {} (existing files are never overwritten)",
                path.display()
            )
        })?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&frame.rgba)?;
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Film {
    pub location: Location,
    pub end_log10_zoom: f64,
    pub seconds: f64,
    pub fps: u32,
    #[serde(default)]
    pub ease: bool,
    #[serde(default = "one")]
    pub temporal_samples: u32,
    #[serde(default = "shutter")]
    pub shutter: f64,
}
fn one() -> u32 {
    1
}
fn shutter() -> f64 {
    0.5
}
impl Film {
    pub fn validate(&self) -> Result<()> {
        Camera::from_location(&self.location)?;
        ensure!(
            self.end_log10_zoom.is_finite() && (-1.0..=280.0).contains(&self.end_log10_zoom),
            "end zoom must be -1..280"
        );
        ensure!(
            self.seconds.is_finite() && self.seconds > 0.0 && self.seconds <= 86400.0,
            "invalid film duration"
        );
        ensure!((1..=240).contains(&self.fps), "fps must be 1..240");
        ensure!(
            (1..=8).contains(&self.temporal_samples),
            "temporal_samples must be 1..8"
        );
        ensure!(
            self.shutter.is_finite() && (0.0..=1.0).contains(&self.shutter),
            "shutter must be 0..1"
        );
        ensure!(
            (self.seconds * self.fps as f64).round() >= 2.0,
            "film must have at least two frames"
        );
        Ok(())
    }
    pub fn zoom(&self, frame_position: f64, count: u32) -> f64 {
        let t = (frame_position / (count - 1) as f64).clamp(0.0, 1.0);
        let t = if self.ease {
            t * t * (3.0 - 2.0 * t)
        } else {
            t
        };
        self.location.log10_zoom + (self.end_log10_zoom - self.location.log10_zoom) * t
    }
}
pub fn export(spec: &Film, settings: &Settings, directory: &Path) -> Result<()> {
    spec.validate()?;
    settings.validate()?;
    if directory.exists() {
        ensure!(
            fs::read_dir(directory)?.next().is_none(),
            "film output directory must be empty"
        );
    }
    fs::create_dir_all(directory)?;
    fs::write(
        directory.join("film.json"),
        serde_json::to_vec_pretty(spec)?,
    )?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(settings.threads)
        .build()?;
    let count = (spec.seconds * spec.fps as f64).round() as u32;
    let mut engine = Engine::default();
    let mut camera = Camera::from_location(&spec.location)?;
    for index in 0..count {
        let mut accumulation = vec![0f32; settings.width as usize * settings.height as usize * 3];
        let mut last = None;
        for temporal in 0..spec.temporal_samples {
            let offset =
                ((temporal as f64 + 0.5) / spec.temporal_samples as f64 - 0.5) * spec.shutter;
            camera.log10_zoom = spec.zoom(index as f64 + offset, count);
            let prepared = engine
                .prepare(&camera, settings, &|| false)
                .context("reference failed")?;
            let frame = pool
                .install(|| prepared.render(&camera, settings, index as u64, 1, true, &|| false))
                .context("render failed")?;
            for (rgb, pixel) in accumulation.chunks_mut(3).zip(frame.rgba.chunks(4)) {
                for channel in 0..3 {
                    rgb[channel] += (pixel[channel] as f32 / 255.0).powf(2.2);
                }
            }
            last = Some(frame);
        }
        let mut frame = last.context("no temporal sample")?;
        for (pixel, rgb) in frame.rgba.chunks_mut(4).zip(accumulation.chunks(3)) {
            for channel in 0..3 {
                pixel[channel] = (255.0
                    * (rgb[channel] / spec.temporal_samples as f32).powf(1.0 / 2.2))
                .round() as u8;
            }
        }
        save_png(&frame, &directory.join(format!("frame-{index:06}.png")))?;
        eprintln!(
            "frame {}/{} · zoom 10^{:.3} · MPFR repairs {} · BLA skipped {}",
            index + 1,
            count,
            camera.log10_zoom,
            frame.stats.repairs,
            frame.stats.skipped
        );
    }
    fs::write(
        directory.join("encode.sh"),
        format!(
            "#!/bin/sh\nset -eu\n# Run inside this frame directory. Requires FFmpeg with libx264.\nffmpeg -n -framerate {} -i frame-%06d.png -c:v libx264 -preset slow -crf 16 -pix_fmt yuv420p -vf 'pad=ceil(iw/2)*2:ceil(ih/2)*2' -movflags +faststart zoom.mp4\n",
            spec.fps
        ),
    )?;
    Ok(())
}
pub fn load_location(path: &Path) -> Result<Location> {
    Ok(serde_json::from_reader(File::open(path)?)?)
}
