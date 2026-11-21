use crate::{
    camera::Camera,
    engine::{Engine, Frame, Settings},
};
use crossbeam_channel::{Receiver, Sender, bounded};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

struct Request {
    generation: u64,
    camera: Camera,
}
pub struct Scheduler {
    requests: Option<Sender<Request>>,
    pending: Receiver<Request>,
    submission: Mutex<()>,
    pub frames: Receiver<Frame>,
    generation: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Scheduler {
    pub fn new(settings: Settings) -> anyhow::Result<Self> {
        Self::start(
            settings,
            #[cfg(feature = "gpu")]
            None,
        )
    }

    pub(crate) fn start(
        settings: Settings,
        #[cfg(feature = "gpu")] mut gpu: Option<crate::gpu::Gpu>,
    ) -> anyhow::Result<Self> {
        settings.validate()?;
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(settings.threads)
            .build()?;
        let (requests, rx) = bounded::<Request>(1);
        let pending = rx.clone();
        let (tx, frames) = bounded::<Frame>(2);
        let generation = Arc::new(AtomicU64::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let current = generation.clone();
        let shutdown = stopped.clone();
        let worker = thread::Builder::new()
            .name("reference-and-render".into())
            .spawn(move || {
                let mut engine = Engine::default();
                while let Ok(mut request) = rx.recv() {
                    for newer in rx.try_iter() {
                        request = newer;
                    }
                    let cancel = || {
                        shutdown.load(Ordering::Acquire)
                            || current.load(Ordering::Acquire) != request.generation
                    };
                    if cancel() {
                        continue;
                    }
                    let mut prepared = None;
                    for divider in [8, 4, 2, 1] {
                        #[cfg(feature = "gpu")]
                        let frame = if let Some(device) = &gpu
                            && crate::gpu::Gpu::supports(&request.camera, &settings)
                        {
                            match device.render(
                                &request.camera,
                                &settings,
                                request.generation,
                                divider,
                                &cancel,
                            ) {
                                Ok(Some(frame)) => Some(frame),
                                Ok(None) => break,
                                Err(error) => {
                                    eprintln!("Falha no cálculo Vulkan, usando CPU: {error:#}");
                                    gpu = None;
                                    None
                                }
                            }
                        } else {
                            None
                        };
                        #[cfg(not(feature = "gpu"))]
                        let frame = None;
                        let frame = frame.or_else(|| {
                            if cancel() {
                                return None;
                            }
                            if prepared.is_none() {
                                prepared = engine.prepare(&request.camera, &settings, &cancel);
                            }
                            let prepared = prepared.as_ref()?;
                            pool.install(|| {
                                prepared.render(
                                    &request.camera,
                                    &settings,
                                    request.generation,
                                    divider,
                                    false,
                                    &cancel,
                                )
                            })
                        });
                        let Some(mut pending) = frame else {
                            break;
                        };
                        // Bounded backpressure without blocking shutdown or newer requests.
                        loop {
                            if cancel() {
                                break;
                            }
                            match tx.send_timeout(pending, Duration::from_millis(10)) {
                                Ok(()) => break,
                                Err(crossbeam_channel::SendTimeoutError::Timeout(frame)) => {
                                    pending = frame
                                }
                                Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => return,
                            }
                        }
                        if cancel() {
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            requests: Some(requests),
            pending,
            submission: Mutex::new(()),
            frames,
            generation,
            stopped,
            worker: Some(worker),
        })
    }
    pub fn submit(&self, camera: Camera) -> u64 {
        let _guard = self
            .submission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        // At most one waiting camera, even during a burst of input events.
        while self.pending.try_recv().is_ok() {}
        if let Some(tx) = &self.requests {
            let _ = tx.try_send(Request { generation, camera });
        }
        generation
    }
    pub fn current(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
}
impl Drop for Scheduler {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
