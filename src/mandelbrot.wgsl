struct Params {
    center_x: f64,
    center_y: f64,
    image_width: f64,
    image_height: f64,
    step: f64,
    width: f64,
    height: f64,
    first_row: u32,
    rows: u32,
    iterations: u32,
    padding: u32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> samples: array<vec2<f64>>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = u32(params.width);
    let index = id.x;
    if index >= width * params.rows {
        return;
    }
    let px = (f64(index % width) + 0.5lf) * params.image_width / params.width;
    let py = (f64(params.first_row + index / width) + 0.5lf) * params.image_height / params.height;
    let cx = params.center_x + (px - params.image_width * 0.5lf) * params.step;
    let cy = params.center_y + (py - params.image_height * 0.5lf) * params.step;
    let shifted = cx - 0.25lf;
    let q = shifted * shifted + cy * cy;
    samples[index] = vec2<f64>(0.0lf, 0.0lf);
    if q * (q + shifted) < 0.25lf * cy * cy - 1e-14lf ||
       (cx + 1.0lf) * (cx + 1.0lf) + cy * cy < 0.0625lf - 1e-14lf {
        return;
    }

    var x = 0.0lf;
    var y = 0.0lf;
    var xx = 0.0lf;
    var yy = 0.0lf;
    for (var n = 1u; n <= params.iterations; n++) {
        y = 2.0lf * x * y + cy;
        x = xx - yy + cx;
        xx = x * x;
        yy = y * y;
        let squared = xx + yy;
        if squared > 256.0lf {
            samples[index] = vec2<f64>(f64(n), squared);
            return;
        }
    }
}
