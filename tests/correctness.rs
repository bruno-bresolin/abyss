use abyss::{
    camera::{Camera, Location},
    engine::{Engine, Settings},
    film::Film,
    math::{self, C, Sample},
    scheduler::Scheduler,
};
use rug::Float;
use std::time::{Duration, Instant};

fn camera(re: &str, im: &str, zoom: f64) -> Camera {
    Camera::from_location(&Location {
        real: re.into(),
        imag: im.into(),
        log10_zoom: zoom,
    })
    .unwrap()
}
fn compare(a: Sample, b: Sample, context: &str) {
    match (a, b) {
        (
            Sample::Escaped {
                iteration: a,
                smooth: x,
            },
            Sample::Escaped {
                iteration: b,
                smooth: y,
            },
        ) => {
            assert_eq!(a, b, "iteration mismatch {context}");
            assert!(
                (x - y).abs() < 0.005,
                "smooth mismatch {context}: {x} vs {y}"
            );
        }
        _ => assert_eq!(a, b, "classification mismatch {context}"),
    }
}
#[test]
fn high_precision_navigation_survives_220_decades() {
    let mut c = camera("-2", "0", 220.0);
    let original = c.re.clone();
    c.pan(1.0, 0.0, 960);
    assert_ne!(original, c.re);
    // The absolute f64 value cannot represent this move, but the camera can.
    assert_eq!(original.to_f64(), c.re.to_f64());
    let offset = Float::with_val(1280, &c.re - &original).to_f64();
    assert!((offset / c.pixel_size(960) - 1.0).abs() < 1e-12);
    let restored = Camera::from_location(&c.location()).unwrap();
    assert_eq!(c.re, restored.re);
}
#[test]
fn perturbation_and_bla_match_independent_mpfr() {
    let cases = [
        ("-0.5", "0", 0.0),
        ("-0.743643887037151", "0.131825904205330", 8.0),
        ("-2", "0", 220.0),
        ("-2", "0", 280.0),
        ("1e-350", "0", 220.0),
        ("0.25", "0", 5.0),
        ("-0.743643887037151", "0.131825904205330", 14.0),
        ("-0.1", "0", 220.0),
    ];
    let mut total_skips = 0;
    for (re, im, zoom) in cases {
        let c = camera(re, im, zoom);
        for bla in [false, true] {
            let s = Settings {
                width: 64,
                height: 48,
                max_iter: 3000,
                threads: 2,
                bla,
                ..Settings::default()
            };
            let mut engine = Engine::default();
            let prepared = engine.prepare(&c, &s, &|| false).unwrap();
            for (x, y) in [
                (0.5, 0.5),
                (17.5, 13.5),
                (32.5, 24.5),
                (63.5, 47.5),
                (31.5, 24.5),
            ] {
                let (actual, skips, _) = prepared.sample(&c, x, y, &s, &|| false);
                total_skips += skips;
                let mut pr = c.re.clone();
                pr += (x - 32.0) * c.pixel_size(64);
                let mut pi = c.im.clone();
                pi += (y - 24.0) * c.pixel_size(64);
                let expected = math::direct(&pr, &pi, s.max_iter, &|| false);
                compare(
                    actual,
                    expected,
                    &format!("{re}+i{im} at 1e{zoom}, BLA={bla}, pixel {x},{y}"),
                );
            }
        }
    }
    assert!(
        total_skips > 0,
        "BLA acceleration must actually execute in this test"
    );
}
#[test]
fn complete_frames_are_opaque_and_cancellation_publishes_nothing() {
    let c = camera("-0.5", "0", 0.0);
    let s = Settings {
        width: 48,
        height: 32,
        max_iter: 128,
        threads: 2,
        ..Settings::default()
    };
    let p = Engine::default().prepare(&c, &s, &|| false).unwrap();
    let frame = p.render(&c, &s, 7, 4, false, &|| false).unwrap();
    assert_eq!(frame.generation, 7);
    assert!(frame.rgba.chunks(4).all(|px| px[3] == 255));
    assert!(p.render(&c, &s, 7, 1, false, &|| true).is_none());
}
#[test]
fn scheduler_survives_burst_and_only_latest_is_accepted() {
    let s = Settings {
        width: 32,
        height: 24,
        max_iter: 128,
        threads: 2,
        ..Settings::default()
    };
    let scheduler = Scheduler::new(s).unwrap();
    for i in 0..30 {
        scheduler.submit(camera("-0.5", "0", i as f64 * 0.01));
    }
    let latest = scheduler.current();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(
            Instant::now() < deadline,
            "latest generation did not finish"
        );
        let frame = scheduler
            .frames
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        if frame.generation == latest && frame.final_pass {
            assert!(frame.rgba.chunks(4).all(|px| px[3] == 255));
            break;
        }
    }
}
#[test]
fn film_path_has_exact_endpoints_and_constant_log_speed() {
    let film = Film {
        location: Location {
            real: "-2".into(),
            imag: "0".into(),
            log10_zoom: 200.0,
        },
        end_log10_zoom: 220.0,
        seconds: 10.0,
        fps: 60,
        ease: false,
        temporal_samples: 2,
        shutter: 0.5,
    };
    film.validate().unwrap();
    assert_eq!(film.zoom(0.0, 600), 200.0);
    assert_eq!(film.zoom(599.0, 600), 220.0);
    assert!((film.zoom(300.0, 600) - film.zoom(299.0, 600) - 20.0 / 599.0).abs() < 1e-12);
}
#[test]
fn unsupported_depth_and_invalid_memory_are_rejected() {
    assert!(
        Camera::from_location(&Location {
            log10_zoom: 301.0,
            ..Location::default()
        })
        .is_err()
    );
    assert!(
        Camera::from_location(&Location {
            real: "NaN".into(),
            ..Location::default()
        })
        .is_err()
    );
    assert!(
        Settings {
            width: 8192,
            height: 8192,
            memory_mb: 128,
            ..Settings::default()
        }
        .validate()
        .is_err()
    );
    assert!(C::new(1e-220, 1e-220).abs() > 0.0);
}

#[test]
fn cancellation_is_repaired_instead_of_colored_as_interior() {
    let c = camera("-0.5", "0", 0.0);
    let s = Settings {
        width: 64,
        height: 48,
        max_iter: 256,
        ..Settings::default()
    };
    let p = Engine::default().prepare(&c, &s, &|| false).unwrap();
    // Offset +0.5 makes the pixel parameter exactly zero, cancelling reference Z_1.
    let (value, _, repaired) = p.sample(&c, 40.0, 24.0, &s, &|| false);
    assert!(repaired);
    assert_eq!(value, Sample::Unresolved);
}

#[test]
fn shutdown_does_not_wait_for_a_full_result_channel() {
    let s = Settings {
        width: 32,
        height: 24,
        max_iter: 128,
        threads: 2,
        ..Settings::default()
    };
    let scheduler = Scheduler::new(s).unwrap();
    scheduler.submit(camera("-0.5", "0", 0.0));
    // Wait until the bounded output is full without consuming it.
    let deadline = Instant::now() + Duration::from_secs(10);
    while scheduler.frames.len() < 2 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let started = Instant::now();
    drop(scheduler);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn shallow_fast_kernel_matches_mpfr_on_a_grid() {
    for yi in 0..11 {
        for xi in 0..15 {
            let re = -2.1 + xi as f64 * 0.21;
            let im = -1.2 + yi as f64 * 0.23;
            compare(
                math::direct_f64(C::new(re, im), 512, &|| false),
                math::direct(
                    &Float::with_val(256, re),
                    &Float::with_val(256, im),
                    512,
                    &|| false,
                ),
                &format!("shallow {re}+i{im}"),
            );
        }
    }
}

#[test]
fn film_exports_complete_pngs_and_preserves_existing_output() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("abyss-film-{}-{unique}", std::process::id()));
    let film = Film {
        location: Location {
            real: "-0.5".into(),
            imag: "0".into(),
            log10_zoom: 0.0,
        },
        end_log10_zoom: 0.05,
        seconds: 0.5,
        fps: 4,
        ease: true,
        temporal_samples: 2,
        shutter: 0.5,
    };
    let settings = Settings {
        width: 16,
        height: 16,
        max_iter: 128,
        threads: 2,
        ..Settings::default()
    };
    abyss::film::export(&film, &settings, &directory).unwrap();
    for index in 0..2 {
        let file = std::fs::File::open(directory.join(format!("frame-{index:06}.png"))).unwrap();
        let mut reader = png::Decoder::new(file).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (16, 16));
        assert!(
            pixels[..info.buffer_size()]
                .chunks(4)
                .all(|pixel| pixel[3] == 255)
        );
    }
    assert!(directory.join("encode.sh").is_file());
    assert!(abyss::film::export(&film, &settings, &directory).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}
