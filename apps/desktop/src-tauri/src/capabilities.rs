//! What the machine can do (`get_capabilities`): GPUs, decoders, encoders
//! and OCR. The command answers at once; the GPU list and the encoder
//! probe (one to two seconds on the first start, cached afterwards in the
//! engine's `caps` folder) run on a background thread after the window
//! opened, and `probe_state` stays `Pending` until they are done.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use blurrd_core::{Capabilities, DecoderInfo, OcrCapability, ProbeState};
use blurrd_video::sys::gpu::{self, Adapter};

use crate::projects::lock;

/// Codecs blurrd decodes.
const CODECS: [&str; 3] = ["h264", "hevc", "av1"];

/// The capabilities, filled in by the background probe.
#[derive(Debug)]
pub(crate) struct CapabilityProbe {
    current: Mutex<Capabilities>,
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
    pub(crate) fn restart(self: &Arc<Self>, caps_dir: PathBuf) {
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
        self.start(caps_dir);
    }

    /// Starts the probe on its own thread: the GPUs from DXGI, the
    /// decoders they offer and the encoder chain (cached in `caps_dir`).
    pub(crate) fn start(self: &Arc<Self>, caps_dir: PathBuf) {
        let probe = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("blurrd-capabilities".to_owned())
            .spawn(move || {
                let adapters = gpu::adapters();
                let decoders = decoders(&adapters);
                let encoders = blurrd_video::probe_encoders(&caps_dir);
                let mut current = lock(&probe.current);
                current.gpus = adapters.into_iter().map(|adapter| adapter.info).collect();
                current.decoders = decoders;
                current.encoders = encoders;
                current.probe_state = ProbeState::Ready;
            });
        if let Err(error) = spawned {
            tauri_plugin_log::log::warn!("could not start the capability probe: {error}");
            lock(&self.current).probe_state = ProbeState::Ready;
        }
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
    use blurrd_core::{OcrCapability, ProbeState};

    use super::{CapabilityProbe, decoders};

    #[test]
    fn capabilities_are_pending_until_probed() {
        let probe = CapabilityProbe::new(OcrCapability::NotProbed);
        let current = probe.current();
        assert_eq!(current.probe_state, ProbeState::Pending);
        assert_eq!(current.encoders, vec![]);
        assert_eq!(current.app_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn software_decoders_exist_without_a_gpu() {
        let list = decoders(&[]);
        assert_eq!(list.len(), 3);
        assert!(list.iter().all(|decoder| !decoder.hardware));
    }
}
