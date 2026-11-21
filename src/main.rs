#![forbid(unsafe_code)]
use abyss::{
    camera::{Camera, Location},
    engine::{Engine, Settings},
    film,
};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{fs::File, path::PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Abyss · SDL3 deep zoom explorer and deterministic film renderer"
)]
struct Cli {
    #[arg(long, default_value_t = 960)]
    width: u32,
    #[arg(long, default_value_t = 640)]
    height: u32,
    #[arg(long, default_value_t = 4096)]
    iterations: u32,
    #[arg(long)]
    threads: Option<usize>,
    #[arg(long, default_value_t = 512)]
    memory_mb: usize,
    #[arg(long)]
    no_bla: bool,
    #[arg(long)]
    location: Option<PathBuf>,
    /// Select an Intel, AMD or NVIDIA Vulkan GPU for the interactive explorer.
    #[cfg(feature = "gpu")]
    #[arg(long, value_enum)]
    gpu: Option<abyss::gpu::GpuVendor>,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// Render a complete supersampled still, without SDL or a display.
    Render {
        #[arg(long)]
        output: PathBuf,
    },
    /// Render a deterministic PNG sequence and an FFmpeg encoding script.
    Film {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Open the interactive SDL3 explorer (the default command).
    Explore,
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    #[cfg(feature = "gpu")]
    if cli.gpu.is_some()
        && matches!(
            cli.command,
            Some(Command::Render { .. } | Command::Film { .. })
        )
    {
        anyhow::bail!("--gpu é uma opção do modo explore");
    }
    let mut settings = Settings {
        width: cli.width,
        height: cli.height,
        max_iter: cli.iterations,
        memory_mb: cli.memory_mb,
        bla: !cli.no_bla,
        ..Settings::default()
    };
    if let Some(threads) = cli.threads {
        settings.threads = threads;
    }
    settings.validate()?;
    match cli.command.unwrap_or(Command::Explore) {
        Command::Film { config, output } => {
            let spec: film::Film = serde_json::from_reader(File::open(config)?)?;
            film::export(&spec, &settings, &output)
        }
        command => {
            let location = if let Some(path) = cli.location {
                film::load_location(&path)?
            } else {
                Location::default()
            };
            let camera = Camera::from_location(&location)?;
            match command {
                Command::Render { output } => {
                    let pool = rayon::ThreadPoolBuilder::new()
                        .num_threads(settings.threads)
                        .build()?;
                    let mut engine = Engine::default();
                    let prepared = engine
                        .prepare(&camera, &settings, &|| false)
                        .context("reference failed")?;
                    let frame = pool
                        .install(|| prepared.render(&camera, &settings, 1, 1, true, &|| false))
                        .context("render failed")?;
                    film::save_png(&frame, &output)?;
                    eprintln!(
                        "{} · zoom 10^{} · repairs {} · BLA skips {} · unresolved samples {}",
                        output.display(),
                        camera.log10_zoom,
                        frame.stats.repairs,
                        frame.stats.skipped,
                        frame.stats.unresolved
                    );
                    Ok(())
                }
                Command::Explore => {
                    #[cfg(feature = "desktop")]
                    {
                        #[cfg(feature = "gpu")]
                        if let Some(vendor) = cli.gpu {
                            return abyss::desktop::run_with_gpu(camera, settings, vendor);
                        }
                        abyss::desktop::run(camera, settings)
                    }
                    #[cfg(not(feature = "desktop"))]
                    {
                        anyhow::bail!(
                            "desktop disabled; build with --features desktop or use render/film"
                        )
                    }
                }
                Command::Film { .. } => unreachable!(),
            }
        }
    }
}
