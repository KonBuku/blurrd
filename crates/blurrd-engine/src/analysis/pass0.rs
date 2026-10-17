//! Pass 0 of the verification during the analysis: the OCR worker that
//! builds the name dictionary and the line log from the protected zones
//! (plan 7.2 step 1).
//!
//! The worker gets every frame's crops (a bounded queue: a slow OCR slows
//! the analysis down instead of dropping crops) and the structure signals
//! the HUD stages found on it. It reads the HUD zones of every
//! [`DictionaryBuilder::CADENCE`]th frame and of every frame with HUD
//! structure (every frame with [`Coverage::EveryFrame`]) and the name zone
//! of a screen surface on every frame its gate makes a candidate. The
//! dictionary and the lines stay in memory for the verification; the
//! counters go into the analysis report.

use std::collections::BTreeMap;
use std::sync::Arc;

use blurrd_core::{Coverage, HudGeometry, Luid, MediaInfo, Region, RegionSet};
use blurrd_ocr::{Ep, GpuAdapter, ModelSet, OcrEngine, OcrOptions, OrtRuntime, Whitelist};
use crossbeam_channel::{Receiver, RecvTimeoutError};

use super::frame::{FrameCrops, OcrReads};
use super::pipeline::POLL;
use crate::dictionary::{DictionaryBuilder, Pass0Stats, ZoneKind};
use crate::engine::OcrSetup;
use crate::verify::LineLog;

/// The OCR side of pass 0, ready to read.
pub(crate) struct Pass0 {
    // The runtime must outlive the engine's sessions.
    _runtime: OrtRuntime,
    ocr: OcrEngine,
    builder: DictionaryBuilder,
    coverage: Coverage,
    log: LineLog,
}

impl std::fmt::Debug for Pass0 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pass0")
            .field("stats", &self.builder.stats())
            .finish_non_exhaustive()
    }
}

/// What the verification needs from a finished pass 0.
pub(crate) struct Pass0Parts<'a> {
    /// The OCR engine, for the blurred reads.
    pub(crate) ocr: &'a mut OcrEngine,
    /// The words that are never names.
    pub(crate) whitelist: &'a Whitelist,
    /// The lines read.
    pub(crate) log: &'a LineLog,
}

impl Pass0 {
    /// Loads ONNX Runtime and the models of `setup` for the video `info`
    /// with the HUD at `geometry`, reading the screen surfaces of
    /// `regions` and the frames of `coverage`. Fails with the reason when
    /// OCR cannot run (the analysis then goes on without it).
    pub(crate) fn open(
        setup: &OcrSetup,
        adapter: Option<Luid>,
        geometry: &HudGeometry,
        info: &MediaInfo,
        regions: RegionSet,
        coverage: Coverage,
    ) -> Result<Self, String> {
        let runtime = OrtRuntime::load(&setup.ort_dylib).map_err(|e| e.to_string())?;
        let models = ModelSet::load(&setup.model_dir).map_err(|e| e.to_string())?;
        let builder = DictionaryBuilder::verifying(geometry, info.width, info.height, regions);
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
        let fps = f64::from(info.fps.num) / f64::from(info.fps.den.max(1));
        let half_second = (fps * 0.5).round().clamp(1.0, 1e6) as u32;
        Ok(Self {
            _runtime: runtime,
            ocr,
            builder,
            coverage,
            log: LineLog::new(half_second),
        })
    }

    /// What the crop workers cut for this pass 0.
    pub(crate) fn reads(&self) -> OcrReads {
        let mut screens = [None; 4];
        for zone in self.builder.zones() {
            if let ZoneKind::Screen(region) = zone.kind
                && let Some(i) = Region::SCREENS.iter().position(|&r| r == region)
            {
                screens[i] = Some(zone.rect);
            }
        }
        OcrReads { screens }
    }

    /// The counters so far.
    pub(crate) fn stats(&self) -> Pass0Stats {
        self.builder.stats()
    }

    /// The frames it read.
    pub(crate) fn coverage(&self) -> Coverage {
        self.coverage
    }

    /// What the verification reads.
    pub(crate) fn parts(&mut self) -> Pass0Parts<'_> {
        Pass0Parts {
            ocr: &mut self.ocr,
            whitelist: self.builder.whitelist(),
            log: &self.log,
        }
    }

    /// Reads the frames from `frames` until it closes or `stopped` says
    /// the analysis stops (an error or a cancellation: the frames still
    /// queued are dropped unread). `structure` delivers `(frame, regions
    /// with structure)` from each of the `hud_stages` HUD stages. Returns
    /// itself with what it read; fails with the reason when OCR fails.
    pub(crate) fn run(
        mut self,
        frames: &Receiver<Arc<FrameCrops>>,
        structure: &Receiver<(u32, RegionSet)>,
        hud_stages: usize,
        stopped: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        // Per frame: signals received, and the regions with structure.
        let mut signals: BTreeMap<u32, (usize, RegionSet)> = BTreeMap::new();
        for crops in frames {
            let index = crops.index;
            while signals.get(&index).is_none_or(|s| s.0 < hud_stages) {
                if stopped() {
                    return Ok(self);
                }
                let (frame, regions) = match structure.recv_timeout(POLL) {
                    Ok(signal) => signal,
                    Err(RecvTimeoutError::Timeout) => continue,
                    // The stages stopped (the analysis is ending early).
                    Err(RecvTimeoutError::Disconnected) => return Ok(self),
                };
                let entry = signals.entry(frame).or_insert((0, RegionSet::empty()));
                entry.0 += 1;
                for region in regions.iter() {
                    entry.1.insert(region);
                }
            }
            let on = signals.remove(&index).map_or(RegionSet::empty(), |s| s.1);
            let hud = self.coverage == Coverage::EveryFrame
                || DictionaryBuilder::wants(index, !on.is_empty());
            if !self.read(&crops, hud, on, stopped)? {
                return Ok(self);
            }
        }
        Ok(self)
    }

    /// Reads the zones of one frame's `crops`: the HUD zones when `hud`
    /// is set, the screen zones when the crops hold them; the frame showed
    /// the HUD structure of `structure`. Returns
    /// `false` when `stopped` said to stop first; fails with the reason
    /// when OCR fails.
    pub(crate) fn read(
        &mut self,
        crops: &FrameCrops,
        hud: bool,
        structure: RegionSet,
        stopped: &dyn Fn() -> bool,
    ) -> Result<bool, String> {
        let mut read = Vec::new();
        for index in 0..self.builder.zones().len() {
            if stopped() {
                return Ok(false);
            }
            let zone = self.builder.zones()[index];
            let hud_zone = matches!(zone.kind, ZoneKind::FeedAndBanner | ZoneKind::LeftPanel);
            if hud_zone && !hud {
                continue;
            }
            // Screen zones have crops only on the frames they are read on.
            if let Some(crop) = crops.ocr_view(zone.kind, zone.rect) {
                read.push(
                    self.builder
                        .push_zone(&mut self.ocr, zone.id, crop)
                        .map_err(|e| e.to_string())?,
                );
            }
        }
        if hud || !read.is_empty() {
            self.builder.count_frame();
            self.log.record(crops.index, hud, structure, read);
        }
        Ok(true)
    }
}
