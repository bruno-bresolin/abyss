use rug::Float;
use std::ops::{Add, Mul, Sub};

#[derive(Clone, Copy, Debug, Default)]
pub struct C {
    pub re: f64,
    pub im: f64,
}
impl C {
    pub const ZERO: Self = Self { re: 0.0, im: 0.0 };
    pub const ONE: Self = Self { re: 1.0, im: 0.0 };
    pub fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    // hypot avoids squaring a 1e-220 delta into zero.
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    pub fn finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
}
impl Add for C {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self::new(self.re + b.re, self.im + b.im)
    }
}
impl Sub for C {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        Self::new(self.re - b.re, self.im - b.im)
    }
}
impl Mul for C {
    type Output = Self;
    fn mul(self, b: Self) -> Self {
        Self::new(
            self.re * b.re - self.im * b.im,
            self.re * b.im + self.im * b.re,
        )
    }
}
impl Mul<f64> for C {
    type Output = Self;
    fn mul(self, b: f64) -> Self {
        Self::new(self.re * b, self.im * b)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sample {
    Escaped { iteration: u32, smooth: f64 },
    Unresolved,
    Cancelled,
}
pub fn escaped(iteration: u32, magnitude: f64) -> Sample {
    Sample::Escaped {
        iteration,
        smooth: iteration as f64 + 1.0 - magnitude.ln().log2(),
    }
}

/// Fast shallow-view kernel. Only selected when pixel spacing dwarfs f64 rounding.
pub fn direct_f64(c: C, max_iter: u32, cancelled: &impl Fn() -> bool) -> Sample {
    let shifted = c.re - 0.25;
    let q = shifted * shifted + c.im * c.im;
    if q * (q + shifted) < 0.25 * c.im * c.im - 1e-14
        || (c.re + 1.0) * (c.re + 1.0) + c.im * c.im < 0.0625 - 1e-14
    {
        return Sample::Unresolved;
    }
    let (mut x, mut y) = (0.0, 0.0);
    let (mut xx, mut yy) = (0.0, 0.0);
    for iteration in 1..=max_iter {
        if iteration % 64 == 0 && cancelled() {
            return Sample::Cancelled;
        }
        y = 2.0 * x * y + c.im;
        x = xx - yy + c.re;
        xx = x * x;
        yy = y * y;
        if xx + yy > 256.0 {
            return escaped(iteration, x.hypot(y));
        }
    }
    Sample::Unresolved
}
/// Independent arbitrary precision oracle and slow repair path. z_0 = 0.
pub fn direct(re: &Float, im: &Float, max_iter: u32, cancelled: &impl Fn() -> bool) -> Sample {
    let p = re.prec().max(im.prec());
    let mut x = Float::with_val(p, 0);
    let mut y = Float::with_val(p, 0);
    for n in 1..=max_iter {
        if n % 64 == 0 && cancelled() {
            return Sample::Cancelled;
        }
        let mut xx = Float::with_val(p, &x * &x);
        let yy = Float::with_val(p, &y * &y);
        let mut xy = Float::with_val(p, &x * &y);
        xy *= 2;
        xy += im;
        xx -= yy;
        xx += re;
        x = xx;
        y = xy;
        let norm = Float::with_val(p, &x * &x) + Float::with_val(p, &y * &y);
        if norm > 256 {
            return escaped(n, x.to_f64().hypot(y.to_f64()));
        }
    }
    Sample::Unresolved
}
