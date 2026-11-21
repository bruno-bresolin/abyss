use crate::{
    camera::Camera,
    math::{self, C, Sample},
    palette,
};
use anyhow::{Result, ensure};
pub(crate) const DIRECT_PIXEL_SIZE: f64 = 1e-9;
use rayon::prelude::*;
use rug::Float;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Debug)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub max_iter: u32,
    pub threads: usize,
    pub memory_mb: usize,
    pub bla: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 960,
            height: 640,
            max_iter: 4096,
            threads: std::thread::available_parallelism()
                .map_or(1, |n| n.get().saturating_sub(2).max(1)),
            memory_mb: 512,
            bla: true,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (16..=8192).contains(&self.width) && (16..=8192).contains(&self.height),
            "dimensions must be 16..8192"
        );
        ensure!(
            (64..=1_000_000).contains(&self.max_iter),
            "iterations must be 64..1000000"
        );
        ensure!((1..=256).contains(&self.threads), "threads must be 1..256");
        ensure!(
            (128..=32768).contains(&self.memory_mb),
            "RAM budget must be 128..32768 MiB"
        );
        let pixels = self.width as usize * self.height as usize;
        // Includes CPU frames, movie samples, orbit, tables and presentation cache reserve.
        ensure!(
            pixels * 32 + (self.max_iter as usize + 1) * 128 < self.memory_mb * 1024 * 1024,
            "RAM budget too small for dimensions/iterations; increase --memory-mb or reduce workload"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Statistics {
    pub repairs: u64,
    pub skipped: u64,
    pub unresolved: u64,
}
#[derive(Clone)]
pub struct Frame {
    pub generation: u64,
    pub camera: Camera,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub stats: Statistics,
    pub final_pass: bool,
}
impl Frame {
    pub fn bytes(&self) -> usize {
        self.rgba.len()
    }
}

pub struct Orbit {
    pub camera: Camera,
    pub precision: u32,
    pub z: Vec<C>,
    tiny: Vec<bool>,
    pub max_iter: u32,
}
impl Orbit {
    pub fn build(camera: &Camera, settings: &Settings, cancel: &impl Fn() -> bool) -> Option<Self> {
        let p = camera.precision(settings.width);
        let re = Float::with_val(p, &camera.re);
        let im = Float::with_val(p, &camera.im);
        let mut x = Float::with_val(p, 0);
        let mut y = Float::with_val(p, 0);
        let mut z = Vec::with_capacity(settings.max_iter as usize + 1);
        let mut tiny = Vec::with_capacity(settings.max_iter as usize + 1);
        z.push(C::ZERO);
        tiny.push(false);
        for n in 0..settings.max_iter {
            if n % 64 == 0 && cancel() {
                return None;
            }
            let mut xx = Float::with_val(p, &x * &x);
            xx -= Float::with_val(p, &y * &y);
            let mut xy = Float::with_val(p, &x * &y);
            xy *= 2;
            xy += &im;
            xx += &re;
            x = xx;
            y = xy;
            let next = C::new(x.to_f64(), y.to_f64());
            tiny.push((x != 0 || y != 0) && next.abs() < 1e-140);
            z.push(next);
            if next.abs() > 16.0 {
                break;
            }
        }
        Some(Self {
            camera: camera.clone(),
            precision: p,
            z,
            tiny,
            max_iter: settings.max_iter,
        })
    }
    fn compatible(&self, camera: &Camera, settings: &Settings) -> bool {
        self.camera.re == camera.re
            && self.camera.im == camera.im
            && self.precision >= camera.precision(settings.width)
            && self.max_iter >= settings.max_iter
    }
}
#[derive(Clone, Copy, Default)]
struct Block {
    a: C,
    b: C,
    radius: f64,
    length: usize,
}
pub struct Bla {
    levels: Vec<Vec<Block>>,
}
impl Bla {
    fn build(orbit: &Orbit, delta_max: f64, cancel: &impl Fn() -> bool) -> Option<Self> {
        let mut first = Vec::with_capacity(orbit.z.len().saturating_sub(1));
        // Start at reference iteration 1. The first step at Z_0=0 is nonlinear.
        for (index, &z) in orbit.z.iter().enumerate().skip(1) {
            if index % 1024 == 0 && cancel() {
                return None;
            }
            let magnitude = z.abs();
            let radius = if (1e-140..1.75).contains(&magnitude) {
                (2.0 * magnitude * 1e-12).min(0.01)
            } else {
                0.0
            };
            first.push(Block {
                a: z * 2.0,
                b: C::ONE,
                radius,
                length: 1,
            });
        }
        let mut levels = vec![first];
        while levels.last().is_some_and(|v| v.len() > 1) {
            if cancel() {
                return None;
            }
            let previous = levels.last().unwrap();
            let mut next = Vec::with_capacity(previous.len() / 2);
            for pair in previous.as_chunks::<2>().0 {
                let x = pair[0];
                let y = pair[1];
                let a = y.a * x.a;
                let b = y.a * x.b + y.b;
                let radius = if x.a.abs() > 0.0 && a.finite() && b.finite() {
                    x.radius
                        .min((y.radius - x.b.abs() * delta_max) / x.a.abs())
                        .max(0.0)
                } else {
                    0.0
                };
                next.push(Block {
                    a,
                    b,
                    radius,
                    length: x.length + y.length,
                });
            }
            levels.push(next);
        }
        Some(Self { levels })
    }
    fn lookup(&self, m: usize, remaining: u32, delta: f64) -> Option<Block> {
        if m == 0 {
            return None;
        }
        let offset = m - 1;
        for (level, blocks) in self.levels.iter().enumerate().rev() {
            let length = 1usize << level;
            if !offset.is_multiple_of(length) {
                continue;
            }
            if let Some(&b) = blocks.get(offset / length)
                && b.length <= remaining as usize
                && delta < b.radius
            {
                return Some(b);
            }
        }
        None
    }
}

pub struct Prepared {
    orbit: Arc<Orbit>,
    bla: Option<Bla>,
}
#[derive(Default)]
pub struct Engine {
    cached: Option<Arc<Orbit>>,
}
impl Engine {
    pub fn prepare(
        &mut self,
        camera: &Camera,
        settings: &Settings,
        cancel: &impl Fn() -> bool,
    ) -> Option<Prepared> {
        let orbit = if let Some(old) = self
            .cached
            .as_ref()
            .filter(|o| o.compatible(camera, settings))
        {
            old.clone()
        } else {
            self.cached = None; // Release an obsolete orbit before allocating its replacement.
            let fresh = Arc::new(Orbit::build(camera, settings, cancel)?);
            self.cached = Some(fresh.clone());
            fresh
        };
        let delta_max = camera.pixel_size(settings.width)
            * (settings.width as f64).hypot(settings.height as f64)
            * 0.51;
        let bla = if settings.bla && delta_max < 1e-9 {
            Some(Bla::build(&orbit, delta_max, cancel)?)
        } else {
            None
        };
        Some(Prepared { orbit, bla })
    }
}
impl Prepared {
    /// None requests an independent MPFR repair; never map a numerical failure to black.
    pub fn perturb(
        &self,
        dc: C,
        max_iter: u32,
        cancel: &impl Fn() -> bool,
    ) -> (Option<Sample>, u64) {
        let mut d = C::ZERO;
        let mut m = 0usize;
        let mut n = 0u32;
        let mut skipped = 0;
        let mut operations = 0u32;
        while n < max_iter {
            operations = operations.wrapping_add(1);
            if operations.is_multiple_of(32) && cancel() {
                return (Some(Sample::Cancelled), skipped);
            }
            let reference = self.orbit.z[m];
            let full = reference + d;
            let magnitude = full.abs();
            if !full.finite() || !d.finite() {
                return (None, skipped);
            }
            if magnitude > 16.0 {
                return (Some(math::escaped(n, magnitude)), skipped);
            }
            // Repair severe cancellation before a rounded sum can silently merge pixels.
            if self.orbit.tiny[m]
                || (m > 0
                    && ((reference.abs() > 0.0 && magnitude < reference.abs() * 1e-7)
                        || (reference.abs() > 0.0 && reference.abs() < 1e-140)))
            {
                return (None, skipped);
            }
            if magnitude < d.abs() || m + 1 >= self.orbit.z.len() {
                d = full;
                m = 0;
            }
            if let Some(block) = self
                .bla
                .as_ref()
                .and_then(|b| b.lookup(m, max_iter - n, d.abs()))
            {
                d = block.a * d + block.b * dc;
                m += block.length;
                n += block.length as u32;
                skipped += block.length as u64;
            } else {
                d = (self.orbit.z[m] * 2.0 + d) * d + dc;
                m += 1;
                n += 1;
            }
        }
        let full = self.orbit.z[m] + d;
        if full.finite() && full.abs() > 16.0 {
            (Some(math::escaped(n, full.abs())), skipped)
        } else if !full.finite() {
            (None, skipped)
        } else {
            (Some(Sample::Unresolved), skipped)
        }
    }
    pub fn sample(
        &self,
        camera: &Camera,
        x: f64,
        y: f64,
        settings: &Settings,
        cancel: &impl Fn() -> bool,
    ) -> (Sample, u64, bool) {
        let step = camera.pixel_size(settings.width);
        let dc = C::new(
            (x - settings.width as f64 * 0.5) * step,
            (y - settings.height as f64 * 0.5) * step,
        );
        let (sample, skipped) = self.perturb(dc, settings.max_iter, cancel);
        if let Some(value) = sample {
            return (value, skipped, false);
        }
        let mut re = camera.re.clone();
        re += dc.re;
        let mut im = camera.im.clone();
        im += dc.im;
        (
            math::direct(&re, &im, settings.max_iter, cancel),
            skipped,
            true,
        )
    }
    /// Tiles own disjoint output slices. Only a complete immutable frame is published.
    pub fn render(
        &self,
        camera: &Camera,
        settings: &Settings,
        generation: u64,
        divider: u32,
        supersample: bool,
        cancel: &(impl Fn() -> bool + Sync),
    ) -> Option<Frame> {
        let width = settings.width.div_ceil(divider);
        let height = settings.height.div_ceil(divider);
        let step = camera.pixel_size(settings.width);
        let shallow = step >= DIRECT_PIXEL_SIZE;
        let center = C::new(camera.re.to_f64(), camera.im.to_f64());
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        let repairs = AtomicU64::new(0);
        let skipped = AtomicU64::new(0);
        let unresolved = AtomicU64::new(0);
        let tile_rows = 8usize;
        rgba.par_chunks_mut(width as usize * 4 * tile_rows)
            .enumerate()
            .for_each(|(tile, rows)| {
                let mut local_stats = Statistics::default();
                for (local, row) in rows.chunks_mut(width as usize * 4).enumerate() {
                    if cancel() {
                        return;
                    }
                    let y = (tile * tile_rows + local) as f64;
                    for (x, pixel) in row.chunks_mut(4).enumerate() {
                        let offsets: &[(f64, f64)] = if supersample {
                            &[(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
                        } else {
                            &[(0.5, 0.5)]
                        };
                        let mut sum = [0f64; 3];
                        for &(sx, sy) in offsets {
                            let px = (x as f64 + sx) * settings.width as f64 / width as f64;
                            let py = (y + sy) * settings.height as f64 / height as f64;
                            let (sample, count, repaired) = if shallow {
                                let c = center
                                    + C::new(
                                        (px - settings.width as f64 * 0.5) * step,
                                        (py - settings.height as f64 * 0.5) * step,
                                    );
                                (math::direct_f64(c, settings.max_iter, cancel), 0, false)
                            } else {
                                self.sample(camera, px, py, settings, cancel)
                            };
                            if sample == Sample::Cancelled {
                                return;
                            }
                            local_stats.repairs += repaired as u64;
                            local_stats.skipped += count;
                            local_stats.unresolved += (sample == Sample::Unresolved) as u64;
                            let color = palette::rgba(sample);
                            for i in 0..3 {
                                sum[i] += (color[i] as f64 / 255.0).powf(2.2);
                            }
                        }
                        for i in 0..3 {
                            pixel[i] = (255.0 * (sum[i] / offsets.len() as f64).powf(1.0 / 2.2))
                                .round() as u8;
                        }
                        pixel[3] = 255;
                    }
                }
                repairs.fetch_add(local_stats.repairs, Ordering::Relaxed);
                skipped.fetch_add(local_stats.skipped, Ordering::Relaxed);
                unresolved.fetch_add(local_stats.unresolved, Ordering::Relaxed);
            });
        if cancel() {
            return None;
        }
        Some(Frame {
            generation,
            camera: camera.clone(),
            width,
            height,
            rgba,
            stats: Statistics {
                repairs: repairs.load(Ordering::Relaxed),
                skipped: skipped.load(Ordering::Relaxed),
                unresolved: unresolved.load(Ordering::Relaxed),
            },
            final_pass: divider == 1,
        })
    }
}
