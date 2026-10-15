//! Pass 0 of the verification during the analysis: the OCR worker that
//! builds the name dictionary from the protected zones (plan 7.2 step 1).
//!
//! The worker gets every frame's crops (a bounded queue: a slow OCR slows
//! the analysis down instead of dropping crops) and the structure signals
//! the HUD stages found on it, and reads the zones of every
//! [`DictionaryBuilder::CADENCE`]th frame and of every frame with HUD
//! structure. The dictionary stays in memory; the counters go into the
//! analysis report.

use std::collections::BTreeMap;
use std::sync::Arc;

use blurrd_core::{HudGeometry, Luid};
use blurrd_ocr::{Ep, GpuAdapter, ModelSet, OcrEngine, OcrOptions, OrtRuntime};
use crossbeam_channel::{Receiver, RecvTimeoutError};

use super::frame::FrameCrops;
use super::pipeline::POLL;
use crate::dictionary::{DictionaryBuilder, Pass0Stats};
use crate::engine::OcrSetup;

/// The OCR side of pass 0, ready to read.
pub(crate) struct Pass0 {
    // The runtime must outlive the engine's sessions.
    _runtime: OrtRuntime,
    ocr: OcrEngine,
    builder: DictionaryBuilder,
}

impl std::fmt::Debug for Pass0 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pass0")
            .field("stats", &self.builder.stats())
            .finish_non_exhaustive()
    }
}

impl Pass0 {
    /// Loads ONNX Runtime and the models of `setup` for a `width ×
    /// height` video with the HUD at `geometry`. Fails with the reason
    /// when OCR cannot run (the analysis then goes on without it).
    pub(crate) fn open(
        setup: &OcrSetup,
        adapter: Option<Luid>,
        geometry: &HudGeometry,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let runtime = OrtRuntime::load(&setup.ort_dylib).map_err(|e| e.to_string())?;
        let models = ModelSet::load(&setup.model_dir).map_err(|e| e.to_string())?;
        let builder = DictionaryBuilder::new(geometry, width, height);
        let adapter = adapter.and_then(|luid| {
            blurrd_video::sys::gpu::dxgi_adapter_index(luid)
                .map(|dxgi_index| GpuAdapter { luid, dxgi_index })
        });
        let options = OcrOptions {
            ep: if setup.gpu { Ep::Auto } else { Ep::Cpu },
            adapter,
            zones: builder.zone_specs(),
            ..OcrOptions::default()
        };
        let ocr = OcrEngine::new(&runtime, &models, &options).map_err(|e| e.to_string())?;
        if let Some(reason) = ocr.fallback() {
            tracing::info!(%reason, "OCR runs on the CPU");
        }
        Ok(Self {
            _runtime: runtime,
            ocr,
            builder,
        })
    }

    /// Reads the frames from `frames` until it closes or `stopped` says
    /// the analysis stops (an error or a cancellation: the frames still
    /// queued are dropped unread). `structure` delivers `(frame,
    /// structure on)` from each of the `hud_stages` HUD stages. Fails
    /// with the reason when OCR fails.
    pub(crate) fn run(
        mut self,
        frames: &Receiver<Arc<FrameCrops>>,
        structure: &Receiver<(u32, bool)>,
        hud_stages: usize,
        stopped: &dyn Fn() -> bool,
    ) -> Result<Pass0Stats, String> {
        // Per frame: signals received, and whether any was on.
        let mut signals: BTreeMap<u32, (usize, bool)> = BTreeMap::new();
        let zones = self.builder.zones().to_vec();
        for crops in frames {
            let index = crops.index;
            while signals.get(&index).is_none_or(|s| s.0 < hud_stages) {
                if stopped() {
                    return Ok(self.builder.stats());
                }
                let (frame, on) = match structure.recv_timeout(POLL) {
                    Ok(signal) => signal,
                    Err(RecvTimeoutError::Timeout) => continue,
                    // The stages stopped (the analysis is ending early).
                    Err(RecvTimeoutError::Disconnected) => return Ok(self.builder.stats()),
                };
                let entry = signals.entry(frame).or_insert((0, false));
                entry.0 += 1;
                entry.1 |= on;
            }
            let on = signals.remove(&index).is_some_and(|s| s.1);
            if !DictionaryBuilder::wants(index, on) {
                continue;
            }
            for zone in &zones {
                if stopped() {
                    return Ok(self.builder.stats());
                }
                if let Some(crop) = crops.ocr_view(zone.kind, zone.rect) {
                    self.builder
                        .push_zone(&mut self.ocr, zone.id, crop)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(self.builder.stats())
    }
}
