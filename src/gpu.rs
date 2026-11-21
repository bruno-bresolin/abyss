mod setup;

use crate::{
    camera::Camera,
    engine::{DIRECT_PIXEL_SIZE, Frame, Settings, Statistics},
    math::{self, Sample},
    palette,
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use clap::ValueEnum;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use wgpu::*;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum GpuVendor {
    Intel,
    Amd,
    Nvidia,
}

impl GpuVendor {
    fn name(self) -> &'static str {
        match self {
            Self::Intel => "Intel",
            Self::Amd => "AMD",
            Self::Nvidia => "NVIDIA",
        }
    }

    fn pci_id(self) -> u32 {
        match self {
            Self::Intel => 0x8086,
            Self::Amd => 0x1002,
            Self::Nvidia => 0x10de,
        }
    }
}

pub(crate) struct Gpu {
    device: Device,
    queue: Queue,
    pipeline: ComputePipeline,
    lost: Arc<AtomicBool>,
}

impl Gpu {
    pub(crate) fn open(vendor: GpuVendor) -> Result<Self> {
        let adapters = || {
            Instance::new(&InstanceDescriptor {
                backends: Backends::VULKAN,
                ..Default::default()
            })
            .enumerate_adapters(Backends::VULKAN)
            .into_iter()
            .filter(|adapter| {
                let info = adapter.get_info();
                info.vendor == vendor.pci_id()
                    && matches!(
                        info.device_type,
                        DeviceType::IntegratedGpu | DeviceType::DiscreteGpu
                    )
            })
            .collect::<Vec<_>>()
        };
        let mut matching = adapters();
        if matching.is_empty() && setup::install_missing()? {
            matching = adapters();
        }
        if matching.is_empty() {
            bail!("nenhuma GPU {} disponível via Vulkan", vendor.name());
        }
        matching.sort_by_key(|adapter| adapter.get_info().device_type != DeviceType::DiscreteGpu);
        let mut errors = Vec::new();
        for adapter in matching {
            let name = adapter.get_info().name;
            match Self::from_adapter(&adapter) {
                Ok(gpu) => {
                    eprintln!("Cálculo Vulkan FP64: {name}");
                    return Ok(gpu);
                }
                Err(error) => errors.push(format!("{name}: {error:#}")),
            }
        }
        bail!("{}", errors.join("; "))
    }

    fn from_adapter(adapter: &Adapter) -> Result<Self> {
        ensure!(
            adapter.features().contains(Features::SHADER_F64),
            "o dispositivo não oferece Vulkan shaderFloat64"
        );
        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            required_features: Features::SHADER_F64,
            ..Default::default()
        }))?;
        let lost = Arc::new(AtomicBool::new(false));
        let lost_callback = lost.clone();
        device.set_device_lost_callback(move |_, _| {
            lost_callback.store(true, Ordering::Release);
        });
        device.push_error_scope(ErrorFilter::Validation);
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Mandelbrot FP64"),
            source: ShaderSource::Wgsl(include_str!("mandelbrot.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Mandelbrot orbits"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = pollster::block_on(device.pop_error_scope()) {
            return Err(error.into());
        }
        Ok(Self {
            device,
            queue,
            pipeline,
            lost,
        })
    }

    pub(crate) fn supports(camera: &Camera, settings: &Settings) -> bool {
        camera.pixel_size(settings.width) >= DIRECT_PIXEL_SIZE && settings.max_iter <= 16_384
    }

    pub(crate) fn render(
        &self,
        camera: &Camera,
        settings: &Settings,
        generation: u64,
        divider: u32,
        cancel: &impl Fn() -> bool,
    ) -> Result<Option<Frame>> {
        if self.lost.load(Ordering::Acquire) {
            bail!("dispositivo Vulkan perdido");
        }
        for filter in [
            ErrorFilter::OutOfMemory,
            ErrorFilter::Internal,
            ErrorFilter::Validation,
        ] {
            self.device.push_error_scope(filter);
        }
        let result = self.render_bands(camera, settings, generation, divider, cancel);
        let mut gpu_error = None;
        for _ in 0..3 {
            if let Some(error) = pollster::block_on(self.device.pop_error_scope()) {
                gpu_error = Some(error);
            }
        }
        if let Some(error) = gpu_error {
            return Err(error.into());
        }
        result
    }

    fn render_bands(
        &self,
        camera: &Camera,
        settings: &Settings,
        generation: u64,
        divider: u32,
        cancel: &impl Fn() -> bool,
    ) -> Result<Option<Frame>> {
        let width = settings.width.div_ceil(divider);
        let height = settings.height.div_ceil(divider);
        // Check cancellation between bounded submissions, even on slow devices.
        let rows_per_band = (8_388_608 / settings.max_iter / width)
            .clamp(1, 64)
            .min(height);
        let size = u64::from(width * rows_per_band) * 16;
        let buffer = |size, usage| {
            self.device.create_buffer(&BufferDescriptor {
                label: None,
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        // Uniform layout matches Params in mandelbrot.wgsl: seven f64s followed by four u32s.
        const BAND_PARAMS_OFFSET: u64 = 7 * 8;
        let params = buffer(
            BAND_PARAMS_OFFSET + 4 * 4,
            BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        );
        let output = buffer(size, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
        let readback = buffer(size, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
        let layout = self.pipeline.get_bind_group_layout(0);
        let group = self.device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let step = camera.pixel_size(settings.width);
        self.queue.write_buffer(
            &params,
            0,
            bytemuck::cast_slice(&[
                camera.re.to_f64(),
                camera.im.to_f64(),
                f64::from(settings.width),
                f64::from(settings.height),
                step,
                f64::from(width),
                f64::from(height),
            ]),
        );
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        let mut stats = Statistics::default();
        for first_row in (0..height).step_by(rows_per_band as usize) {
            if cancel() {
                return Ok(None);
            }
            let rows = rows_per_band.min(height - first_row);
            let pixels = width * rows;
            self.queue.write_buffer(
                &params,
                BAND_PARAMS_OFFSET,
                bytemuck::cast_slice(&[first_row, rows, settings.max_iter, 0]),
            );
            let mut encoder = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(pixels.div_ceil(64), 1, 1);
            }
            let bytes = u64::from(pixels) * 16;
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
            self.queue.submit([encoder.finish()]);
            let slice = readback.slice(..bytes);
            let (send, receive) = mpsc::channel();
            slice.map_async(MapMode::Read, move |result| {
                let _ = send.send(result);
            });
            self.device
                .poll(PollType::Wait)
                .context("aguardando Vulkan")?;
            receive
                .try_recv()
                .context("Vulkan não retornou o buffer")??;
            {
                let data = slice.get_mapped_range();
                let samples = bytemuck::try_cast_slice::<u8, [f64; 2]>(&data)
                    .map_err(|error| anyhow!("buffer Vulkan inválido: {error}"))?;
                for &[iteration, squared] in samples {
                    let sample = if iteration == 0.0 {
                        stats.unresolved += 1;
                        Sample::Unresolved
                    } else {
                        math::escaped(iteration as u32, squared.sqrt())
                    };
                    rgba.extend_from_slice(&palette::rgba(sample));
                }
            }
            readback.unmap();
        }
        Ok((!cancel()).then(|| Frame {
            generation,
            camera: camera.clone(),
            width,
            height,
            rgba,
            stats,
            final_pass: divider == 1,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{camera::Location, engine::Engine};

    #[test]
    fn shader_validates_for_fp64() -> Result<()> {
        let shader = naga::front::wgsl::parse_str(include_str!("mandelbrot.wgsl"))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::FLOAT64,
        )
        .validate(&shader)?;
        Ok(())
    }

    #[test]
    fn vulkan_matches_cpu_across_bands() -> Result<()> {
        let instance = Instance::new(&InstanceDescriptor {
            backends: Backends::VULKAN,
            ..Default::default()
        });
        let Some(adapter) = instance
            .enumerate_adapters(Backends::VULKAN)
            .into_iter()
            .find(|adapter| adapter.features().contains(Features::SHADER_F64))
        else {
            ensure_no_required_test_adapter()?;
            return Ok(());
        };
        eprintln!("Testando shader em {}", adapter.get_info().name);
        let gpu = Gpu::from_adapter(&adapter)?;
        let camera = Camera::from_location(&Location::default())?;
        let settings = Settings {
            width: 37,
            height: 71,
            max_iter: 128,
            ..Settings::default()
        };
        let cancel = || false;
        let cpu = Engine::default()
            .prepare(&camera, &settings, &cancel)
            .unwrap();
        for divider in [8, 1] {
            let expected = cpu
                .render(&camera, &settings, 1, divider, false, &cancel)
                .unwrap();
            let actual = gpu
                .render(&camera, &settings, 1, divider, &cancel)?
                .unwrap();
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height)
            );
            assert_eq!(actual.stats.unresolved, expected.stats.unresolved);
            assert_eq!(actual.rgba, expected.rgba);
        }
        assert!(gpu.render(&camera, &settings, 1, 1, &|| true)?.is_none());
        Ok(())
    }

    fn ensure_no_required_test_adapter() -> Result<()> {
        anyhow::ensure!(
            std::env::var_os("ABYSS_REQUIRE_VULKAN_TEST").is_none(),
            "test requires a Vulkan FP64 adapter"
        );
        eprintln!("Teste de execução ignorado: Vulkan FP64 indisponível");
        Ok(())
    }
}
