use crate::{
    camera::Camera,
    engine::{Frame, Settings},
    scheduler::Scheduler,
};
use anyhow::{Context, Result};
use sdl3::{
    event::Event,
    keyboard::Keycode,
    mouse::MouseButton,
    pixels::{Color, PixelFormat},
    render::{BlendMode, FRect, ScaleMode, Texture},
};
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Layer<'a> {
    frame: Frame,
    texture: Texture<'a>,
    arrived: Instant,
}

pub fn run(camera: Camera, settings: Settings) -> Result<()> {
    run_explorer(
        camera,
        settings,
        #[cfg(feature = "gpu")]
        None,
    )
}

#[cfg(feature = "gpu")]
pub fn run_with_gpu(
    camera: Camera,
    settings: Settings,
    vendor: crate::gpu::GpuVendor,
) -> Result<()> {
    let gpu = match crate::gpu::Gpu::open(vendor) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("GPU indisponível, usando CPU: {error:#}");
            None
        }
    };
    run_explorer(camera, settings, gpu)
}

fn run_explorer(
    mut camera: Camera,
    settings: Settings,
    #[cfg(feature = "gpu")] gpu: Option<crate::gpu::Gpu>,
) -> Result<()> {
    let sdl = sdl3::init().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let video = sdl.video().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    sdl3::hint::set("SDL_RENDER_VSYNC", "1");
    let window = video
        .window("ABYSS / deep zoom", settings.width, settings.height)
        .position_centered()
        .build()?;
    let mut canvas = window.into_canvas();
    eprintln!("SDL3 presentation backend: {}", canvas.renderer_name);
    let creator = canvas.texture_creator();
    let mut events = sdl
        .event_pump()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let scheduler = Scheduler::start(
        settings.clone(),
        #[cfg(feature = "gpu")]
        gpu,
    )?;
    let initial = camera.clone();
    let mut submitted = camera.clone();
    scheduler.submit(camera.clone());
    let mut awaiting = true;
    let mut layers: VecDeque<Layer<'_>> = VecDeque::new();
    let cache_budget = settings.memory_mb * 1024 * 1024 / 4;
    let mut cache_bytes = 0usize;
    let mut auto = false;
    let mut speed = 0.12f64;
    let mut target_zoom = camera.log10_zoom;
    let mut anchor = (settings.width as f64 * 0.5, settings.height as f64 * 0.5);
    let mut dirty_since: Option<Instant> = None;
    let mut last = Instant::now();
    let mut last_title = Instant::now();
    let mut show_help = true;
    let mut frame_count = 0usize;
    // A bounded smoke run is useful for automated SDL dummy-driver testing.
    let smoke_frames = std::env::var("ABYSS_SMOKE_FRAMES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok());
    'running: loop {
        let tick = Instant::now();
        let dt = tick.duration_since(last).as_secs_f64().min(0.05);
        last = tick;
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                Event::KeyDown {
                    keycode: Some(Keycode::Space),
                    repeat: false,
                    ..
                } => {
                    auto = !auto;
                    anchor = (settings.width as f64 * 0.5, settings.height as f64 * 0.5);
                }
                Event::KeyDown {
                    keycode: Some(Keycode::H),
                    repeat: false,
                    ..
                } => show_help = !show_help,
                Event::KeyDown {
                    keycode: Some(Keycode::Equals),
                    ..
                } => speed = (speed * 1.25).min(1.0),
                Event::KeyDown {
                    keycode: Some(Keycode::Minus),
                    ..
                } => speed = (speed / 1.25).max(0.005),
                Event::KeyDown {
                    keycode: Some(Keycode::R),
                    repeat: false,
                    ..
                } => {
                    camera = initial.clone();
                    target_zoom = camera.log10_zoom;
                    auto = false;
                    scheduler.submit(camera.clone());
                    submitted = camera.clone();
                    awaiting = true;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::S),
                    repeat: false,
                    ..
                } => {
                    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
                    let path = format!("location-{stamp}.json");
                    let file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)?;
                    serde_json::to_writer_pretty(file, &camera.location())?;
                    eprintln!("Saved {path}");
                }
                Event::MouseWheel {
                    y,
                    mouse_x,
                    mouse_y,
                    ..
                } => {
                    auto = false;
                    target_zoom = (target_zoom + y as f64 * 0.13).clamp(-1.0, 280.0);
                    anchor = (mouse_x as f64, mouse_y as f64);
                    dirty_since = Some(tick);
                }
                Event::MouseMotion {
                    mousestate,
                    xrel,
                    yrel,
                    ..
                } if mousestate.left() => {
                    auto = false;
                    camera.pan(-(xrel as f64), -(yrel as f64), settings.width);
                    dirty_since = Some(tick);
                }
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    clicks: 2,
                    x,
                    y,
                    ..
                } => {
                    camera.pan(
                        x as f64 - settings.width as f64 * 0.5,
                        y as f64 - settings.height as f64 * 0.5,
                        settings.width,
                    );
                    anchor = (settings.width as f64 * 0.5, settings.height as f64 * 0.5);
                    target_zoom = (camera.log10_zoom + 0.5).min(280.0);
                    dirty_since = Some(tick);
                }
                _ => {}
            }
        }
        if auto {
            // Slow computation limits how far the preview can outrun its source.
            let ceiling = (submitted.log10_zoom + 0.24).min(280.0);
            target_zoom = (target_zoom + speed * dt).min(ceiling);
            anchor = (settings.width as f64 * 0.5, settings.height as f64 * 0.5);
        }
        let difference = target_zoom - camera.log10_zoom;
        if difference.abs() > 1e-7 {
            camera.zoom_at(
                difference * (1.0 - (-dt / 0.075).exp()),
                anchor.0,
                anchor.1,
                settings.width,
                settings.height,
            );
        }
        if let Some(since) = dirty_since
            && tick.duration_since(since) > Duration::from_millis(100)
            && difference.abs() < 0.008
        {
            scheduler.submit(camera.clone());
            submitted = camera.clone();
            awaiting = true;
            dirty_since = None;
        }
        if auto && !awaiting && (camera.log10_zoom - submitted.log10_zoom).abs() > 0.10 {
            scheduler.submit(camera.clone());
            submitted = camera.clone();
            awaiting = true;
        }
        while let Ok(frame) = scheduler.frames.try_recv() {
            if frame.generation != scheduler.current() {
                continue;
            }
            if frame.final_pass {
                awaiting = false;
            }
            let mut texture = creator
                .create_texture_streaming(PixelFormat::RGBA32, frame.width, frame.height)
                .context("create SDL3 streaming texture")?;
            texture.update(None, &frame.rgba, frame.width as usize * 4)?;
            texture.set_scale_mode(ScaleMode::Linear);
            texture.set_blend_mode(BlendMode::Blend);
            cache_bytes += frame.bytes();
            layers.push_back(Layer {
                frame,
                texture,
                arrived: tick,
            });
            // Keep two complete layers for transitions. The validated memory reserve covers these.
            while layers.len() > 2 && (cache_bytes > cache_budget || layers.len() > 8) {
                if let Some(old) = layers.pop_front() {
                    cache_bytes -= old.frame.bytes();
                }
            }
        }
        canvas.set_draw_color(Color::RGB(16, 22, 34));
        canvas.clear();
        let layer_count = layers.len();
        for (index, layer) in layers.iter_mut().enumerate() {
            let rect = camera.projection(&layer.frame.camera, settings.width, settings.height);
            if rect.iter().all(|v| v.is_finite() && v.abs() < 1e8)
                && rect[2] > 0.001
                && rect[3] > 0.001
            {
                let fade = if index + 1 == layer_count && layer_count > 1 {
                    (tick.duration_since(layer.arrived).as_secs_f64() / 0.16).min(1.0)
                } else {
                    1.0
                };
                layer.texture.set_alpha_mod((255.0 * fade) as u8);
                canvas.copy(
                    &layer.texture,
                    None,
                    FRect::new(rect[0], rect[1], rect[2], rect[3]),
                )?;
            }
        }
        canvas.set_blend_mode(BlendMode::Blend);
        canvas.set_draw_color(Color::RGBA(9, 13, 23, 220));
        canvas.fill_rect(FRect::new(
            18.0,
            18.0,
            420.0,
            if show_help { 109.0 } else { 65.0 },
        ))?;
        canvas.set_draw_color(Color::RGB(126, 220, 228));
        canvas.draw_debug_text("A B Y S S   /   DEEP ZOOM", (30.0, 30.0))?;
        canvas.set_draw_color(Color::RGB(224, 231, 240));
        let status = if layers.is_empty() {
            "building first reference"
        } else if awaiting {
            "refining / preview retained"
        } else {
            "ready"
        };
        canvas.draw_debug_text(
            &format!("10^{:.3}  |  {}", camera.log10_zoom, status),
            (30.0, 51.0),
        )?;
        if show_help {
            canvas.set_draw_color(Color::RGB(158, 175, 194));
            canvas.draw_debug_text("Wheel zoom / drag pan / double click focus", (30.0, 74.0))?;
            canvas.draw_debug_text("Space film motion / +/- speed / S bookmark", (30.0, 91.0))?;
            canvas.draw_debug_text("R reset / H hide / Esc exit", (30.0, 108.0))?;
        }
        if smoke_frames.is_some_and(|limit| frame_count + 1 >= limit) {
            if let Ok(path) = std::env::var("ABYSS_SMOKE_CAPTURE") {
                canvas.read_pixels(None)?.save_bmp(path)?;
            }
            eprintln!(
                "SDL smoke: {} complete layers, awaiting={awaiting}",
                layers.len()
            );
        }
        canvas.present();
        if last_title.elapsed() > Duration::from_millis(500) {
            canvas.window_mut().set_title(&format!(
                "ABYSS | 10^{:.3} | {} | {:.2} decades/s | {} MiB cache",
                camera.log10_zoom,
                status,
                speed,
                cache_bytes / 1024 / 1024
            ))?;
            last_title = tick;
        }
        frame_count += 1;
        if smoke_frames.is_some_and(|limit| frame_count >= limit) {
            break;
        }
        let elapsed = tick.elapsed();
        if elapsed < Duration::from_secs_f64(1.0 / 60.0) {
            std::thread::sleep(Duration::from_secs_f64(1.0 / 60.0) - elapsed);
        }
    }
    Ok(())
}
