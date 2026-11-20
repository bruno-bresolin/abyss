use crate::math::Sample;
/// Fixed palette: no per-frame histogram pumping. Unresolved is distinct from loading.
pub fn rgba(sample: Sample) -> [u8; 4] {
    match sample {
        Sample::Escaped { smooth, .. } => {
            let t = smooth * 0.018;
            let channel = |phase: f64| {
                let linear = 0.5 + 0.5 * (std::f64::consts::TAU * (t + phase)).cos();
                (255.0 * linear.powf(1.0 / 2.2)).round() as u8
            };
            [channel(0.02), channel(0.32), channel(0.62), 255]
        }
        Sample::Unresolved => [9, 13, 24, 255],
        Sample::Cancelled => [25, 33, 49, 255],
    }
}
