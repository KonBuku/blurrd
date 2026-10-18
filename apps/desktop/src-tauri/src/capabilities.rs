//! What the machine can do (`get_capabilities`): GPUs, decoders, encoders
//! and OCR. The command answers at once; the GPU list and the encoder
//! probe (one to two seconds on the first start, cached afterwards in the
//! engine's `caps` folder) run on a background thread after the window
//! opened, and `probe_state` stays `Pending` until they are done.
//!
//! The probe encodes on the GPU, so it never runs beside a job: a job
//! that comes up while a probe runs (also the one at start) waits until
//! the probe is done, a second probe is refused, and "Measure again"
//! starts only when no job runs or waits.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

use blurrd_core::{AppError, Capabilities, DecoderInfo, ErrorKind, OcrCapability, ProbeState};
use blurrd_video::sys::gpu::{self, Adapter};

use crate::jobs::Work;
use crate::projects::lock;

/// Codecs blurrd decodes.
const CODECS: [&str; 3] = ["h264", "hevc", "av1"];

/// The capabilities, filled in by the background probe.
#[derive(Debug)]
pub(crate) struct CapabilityProbe {
    current: Mutex<Capabilities>,
    /// Whether a probe runs. Jobs check it before they start working, and
    /// "Measure again" checks that no job runs or waits while holding it.
    running: Mutex<bool>,
    /// Signalled when a probe is done.
    done: Condvar,
}

impl CapabilityProbe {
    /// Capabilities before the probe: only the versions and the OCR state
    /// are known.
    pub(crate) fn new(ocr: OcrCapability) -> Self {
        Self {
            current: Mutex::new(Capabilities {
                gpus: Vec::new(),
                decoders: Vec::new(),
                encoders: Vec::new(),
                probe_state: ProbeState::Pending,
                ocr,
                ffmpeg_version: blurrd_video::ffmpeg_version(),
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
            }),
            running: Mutex::new(false),
            done: Condvar::new(),
        }
    }

    /// `work` that first waits until a running probe is done.
    pub(crate) fn after_probe(self: &Arc<Self>, work: Work) -> Work {
        let probe = Arc::clone(self);
        Box::new(move |sink| {
            probe.wait();
            work(sink);
        })
    }

    /// Waits until no probe runs.
    fn wait(&self) {
        let mut running = lock(&self.running);
        while *running {
            running = self
                .done
                .wait(running)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// The capabilities as far as they are known.
    pub(crate) fn current(&self) -> Capabilities {
        lock(&self.current).clone()
    }

    /// Forgets the cached decoder benchmark and encoder probe in
    /// `caps_dir` and probes again (Settings › Processing "Measure
    /// again"); `probe_state` is `Pending` until the new probe is done.
    /// The decoder benchmark runs again per video on the next job.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] while a probe runs or when `is_idle` (asked
    /// while no job can start) answers that a job runs or waits.
    pub(crate) fn restart(
        self: &Arc<Self>,
        caps_dir: PathBuf,
        is_idle: impl FnOnce() -> bool,
    ) -> Result<(), AppError> {
        let mut running = lock(&self.running);
        if *running {
            return Err(AppError::new(
                ErrorKind::Internal,
                "blurrd is already measuring this PC.",
            ));
        }
        if !is_idle() {
            return Err(AppError::new(
                ErrorKind::Internal,
                "Wait until the running jobs are done.",
            ));
        }
        *running = true;
        drop(running);
        if let Err(error) = std::fs::remove_dir_all(&caps_dir)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tauri_plugin_log::log::warn!("could not clear the capability cache: {error}");
        }
        {
            let mut current = lock(&self.current);
            current.probe_state = ProbeState::Pending;
            current.encoders.clear();
        }
        self.spawn(caps_dir);
        Ok(())
    }

    /// Starts the probe on its own thread: the GPUs from DXGI, the
    /// decoders they offer and the encoder chain (cached in `caps_dir`).
    /// Does nothing while a probe runs.
    pub(crate) fn start(self: &Arc<Self>, caps_dir: PathBuf) {
        {
            let mut running = lock(&self.running);
            if *running {
                return;
            }
            *running = true;
        }
        self.spawn(caps_dir);
    }

    /// Runs the probe on its own thread; `running` is set and is cleared
    /// when the probe is done.
    fn spawn(self: &Arc<Self>, caps_dir: PathBuf) {
        let probe = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("blurrd-capabilities".to_owned())
            .spawn(move || {
                let adapters = gpu::adapters();
                let decoders = decoders(&adapters);
                let encoders = blurrd_video::probe_encoders(&caps_dir);
                probe.finish(|current| {
                    current.gpus = adapters.into_iter().map(|adapter| adapter.info).collect();
                    current.decoders = decoders;
                    current.encoders = encoders;
                });
            });
        if let Err(error) = spawned {
            tauri_plugin_log::log::warn!("could not start the capability probe: {error}");
            self.finish(|_| {});
        }
    }

    /// Ends a probe: `fill` stores its results, the state turns `Ready`
    /// and jobs may start again.
    fn finish(&self, fill: impl FnOnce(&mut Capabilities)) {
        {
            let mut current = lock(&self.current);
            fill(&mut current);
            current.probe_state = ProbeState::Ready;
        }
        *lock(&self.running) = false;
        self.done.notify_all();
    }
}

/// The decoders blurrd can use: software decoding of every codec, plus
/// NVDEC on an NVIDIA GPU or D3D11VA on another GPU (the high-performance
/// adapter, which the decoder choice uses).
fn decoders(adapters: &[Adapter]) -> Vec<DecoderInfo> {
    let hardware = adapters.first().map(|adapter| {
        if adapter.is_nvidia() {
            "nvdec"
        } else {
            "d3d11va"
        }
    });
    CODECS
        .iter()
        .flat_map(|&codec| {
            let software = DecoderInfo {
                codec: codec.to_owned(),
                name: "software".to_owned(),
                hardware: false,
                ten_bit: true,
            };
            let gpu = hardware.map(|name| DecoderInfo {
                codec: codec.to_owned(),
                name: name.to_owned(),
                hardware: true,
                ten_bit: codec != "h264",
            });
            std::iter::once(software).chain(gpu)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use blurrd_core::{OcrCapability, ProbeState};

    use super::{CapabilityProbe, decoders};
    use crate::projects::lock;

    #[test]
    fn capabilities_are_pending_until_probed() {
        let probe = CapabilityProbe::new(OcrCapability::NotProbed);
        let current = probe.current();
        assert_eq!(current.probe_state, ProbeState::Pending);
        assert_eq!(current.encoders, vec![]);
        assert_eq!(current.app_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn jobs_wait_for_a_running_probe_and_a_second_probe_is_refused() {
        let probe = Arc::new(CapabilityProbe::new(OcrCapability::NotProbed));
        *lock(&probe.running) = true;
        let dir = std::env::temp_dir().join(format!("blurrd-caps-{}", std::process::id()));
        assert!(probe.restart(dir.clone(), || true).is_err());

        let (sent, received) = mpsc::channel();
        let waiting = Arc::clone(&probe);
        let worker = std::thread::spawn(move || {
            waiting.wait();
            sent.send(()).unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(200)).is_err());
        probe.finish(|_| {});
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
        assert_eq!(probe.current().probe_state, ProbeState::Ready);

        // Measure again waits for the jobs.
        assert!(probe.restart(dir, || false).is_err());
        assert!(!*lock(&probe.running));
    }

    #[test]
    fn software_decoders_exist_without_a_gpu() {
        let list = decoders(&[]);
        assert_eq!(list.len(), 3);
        assert!(list.iter().all(|decoder| !decoder.hardware));
    }
}
