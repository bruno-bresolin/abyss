use anyhow::{Result, ensure};
use rug::Float;
use serde::{Deserialize, Serialize};

/// Decimal strings are authoritative. Never round a center through f64.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Location {
    pub real: String,
    pub imag: String,
    pub log10_zoom: f64,
}
impl Default for Location {
    fn default() -> Self {
        Self {
            real: "-0.743643887037151".into(),
            imag: "0.131825904205330".into(),
            log10_zoom: 0.0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Camera {
    pub re: Float,
    pub im: Float,
    pub log10_zoom: f64,
}
impl Camera {
    pub fn from_location(location: &Location) -> Result<Self> {
        ensure!(
            location.log10_zoom.is_finite() && (-1.0..=280.0).contains(&location.log10_zoom),
            "zoom must be between 10^-1 and 10^280"
        );
        // Retain guard bits and enough precision for the entire supported navigation range.
        let precision = 1280;
        let re = Float::with_val(precision, Float::parse(&location.real)?);
        let im = Float::with_val(precision, Float::parse(&location.imag)?);
        ensure!(re.is_finite() && im.is_finite(), "center must be finite");
        ensure!(
            re.clone().abs() <= 4 && im.clone().abs() <= 4,
            "center outside supported [-4,4] square"
        );
        Ok(Self {
            re,
            im,
            log10_zoom: location.log10_zoom,
        })
    }
    pub fn location(&self) -> Location {
        Location {
            real: self.re.to_string_radix(10, None),
            imag: self.im.to_string_radix(10, None),
            log10_zoom: self.log10_zoom,
        }
    }
    pub fn precision(&self, width: u32) -> u32 {
        (((self.log10_zoom.max(0.0) * std::f64::consts::LOG2_10 + (width as f64).log2() + 192.0)
            .ceil() as u32)
            .max(256))
        .div_ceil(128)
            * 128
    }
    pub fn pixel_size(&self, width: u32) -> f64 {
        4.0 * 10_f64.powf(-self.log10_zoom) / width as f64
    }
    pub fn pan(&mut self, dx: f64, dy: f64, width: u32) {
        let size = self.pixel_size(width);
        self.re += Float::with_val(self.re.prec(), dx * size);
        self.im += Float::with_val(self.im.prec(), dy * size);
    }
    pub fn zoom_at(&mut self, delta: f64, x: f64, y: f64, width: u32, height: u32) {
        let before = self.pixel_size(width);
        self.log10_zoom = (self.log10_zoom + delta).clamp(-1.0, 280.0);
        let difference = before - self.pixel_size(width);
        self.re += Float::with_val(self.re.prec(), (x - width as f64 * 0.5) * difference);
        self.im += Float::with_val(self.im.prec(), (y - height as f64 * 0.5) * difference);
    }
    /// GPU coordinates derived from a high precision difference, never absolute f64 centers.
    pub fn projection(&self, old: &Self, width: u32, height: u32) -> [f32; 4] {
        let scale = 10_f64.powf(self.log10_zoom - old.log10_zoom);
        let step = self.pixel_size(width);
        let dx = Float::with_val(1280, &old.re - &self.re).to_f64() / step;
        let dy = Float::with_val(1280, &old.im - &self.im).to_f64() / step;
        [
            ((1.0 - scale) * width as f64 * 0.5 + dx) as f32,
            ((1.0 - scale) * height as f64 * 0.5 + dy) as f32,
            (width as f64 * scale) as f32,
            (height as f64 * scale) as f32,
        ]
    }
}
