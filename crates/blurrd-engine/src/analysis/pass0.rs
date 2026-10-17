//! Pass 0 of the verification during the analysis: the OCR workers that
//! build the name dictionary and the line log from the protected zones
//! (plan 7.2 step 1).
//!
//! Pass 0 gets every frame's crops (a bounded queue: a slow OCR slows the
//! analysis down instead of dropping crops) and the structure signals the
//! HUD stages found on it. Text that stays where it is reads the same on
//! every frame, so pass 0 reads a zone at its events ([`ReadSchedule`]):
//! when the layout of its HUD structure changes (a kill-feed row comes,
//! moves or goes, a speaker starts, a chat line arrives, a banner or a
//! screen surface opens), once more when the change has settled, and at
//! least every second in between, which also finds names no detector
//! saw. With [`Coverage::EveryFrame`] it reads the HUD zones of every
//! frame. The whole frame is read every second.
//!
//! An analysis reads the zones with several OCR engines at once
//! ([`Reader`]s: the kill feed and banner, the left panel, the screen
//! surfaces, the full frame), each on its own thread, and every reader
//! recognises the lines of the frames waiting for it in one go. The
//! analysis does not wait for the engines: they are built while the video
//! is probed and calibrated and while the detectors already run
//! ([`Opening`]), and the zones due in the meantime wait in the readers'
//! queues. The dictionary and the lines stay in memory for the
//! verification; the counters go into the analysis report.

use std::collections::BTreeMap;
use std::sync::Arc;

use blurrd_core::{Coverage, HudGeometry, HudLayout, Luid, MediaInfo, Region, RegionSet};
use blurrd_ocr::{
    Ep, GpuAdapter, ModelSet, OcrEngine, OcrOptions, OrtRuntime, Whitelist, ZoneSpec,
};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded};

use super::frame::{FrameCrops, OcrReads};
use super::pipeline::POLL;
use super::stages::{LAYOUT_REGIONS, Structure};
use crate::dictionary::{DictionaryBuilder, Pass0Stats, ZoneKind};
use crate::engine::OcrSetup;
use crate::verify::{HudRead, LineLog};

/// Frames a reader recognises together at most: the lines of a few frames
/// fill the recognition batches.
const BATCH_FRAMES: usize = 8;

/// Frames waiting for each reader of an analysis at most: enough for the
/// zones the analysis queues while the engines are still being built
/// (about a second at several hundred frames per second, a few dozen
/// reads per reader), so the detectors do not wait for them.
const READER_QUEUE: usize = 64;

/// The OCR side of pass 0, ready to read.
pub(crate) struct Pass0 {
    // The runtime must outlive the engines' sessions.
    _runtime: OrtRuntime,
    /// The engines and the zones each reads (at least one).
    readers: Vec<Reader>,
    coverage: Coverage,
    log: LineLog,
    /// Frames whose zones were read.
    frames: u64,
}

impl std::fmt::Debug for Pass0 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pass0")
            .field("readers", &self.readers.len())
            .field("stats", &self.stats())
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

/// One OCR engine (its own sessions, so readers run at the same time) and
/// the protected zones it reads.
struct Reader {
    ocr: OcrEngine,
    builder: DictionaryBuilder,
}

/// The zones of one frame a reader reads, queued for its thread.
struct Task {
    crops: Arc<FrameCrops>,
    /// Indices into the reader's zones.
    zones: Vec<usize>,
    /// The HUD zones among them (the frame then counts as checked).
    hud: HudRead,
}

/// The zones of one frame a reader reads now.
#[derive(Clone, Copy)]
struct ZoneRead<'a> {
    crops: &'a FrameCrops,
    /// Indices into the reader's zones.
    zones: &'a [usize],
    /// The HUD zones among them (the frame then counts as checked).
    hud: HudRead,
}

impl Task {
    fn as_read(&self) -> ZoneRead<'_> {
        ZoneRead {
            crops: &self.crops,
            zones: &self.zones,
            hud: self.hud,
        }
    }
}

impl Reader {
    /// The kinds of the reader's zones, in order.
    fn kinds(&self) -> Vec<ZoneKind> {
        self.builder.zones().iter().map(|z| z.kind).collect()
    }

    /// Reads the zones of `tasks` (frames in order) and records their
    /// lines in `log`; fails with the reason when OCR fails.
    fn read(&mut self, tasks: &[ZoneRead<'_>], log: &mut LineLog) -> Result<(), String> {
        let mut crops = Vec::new();
        let mut owners = Vec::new();
        for (owner, task) in tasks.iter().enumerate() {
            for &i in task.zones {
                let zone = self.builder.zones()[i];
                if let Some(crop) = task.crops.ocr_view(zone.kind, zone.rect) {
                    crops.push((zone.id, crop));
                    owners.push(owner);
                }
            }
        }
        let lines = self
            .builder
            .push_zones(&mut self.ocr, &crops)
            .map_err(|e| e.to_string())?;
        let mut read: Vec<Vec<_>> = tasks.iter().map(|_| Vec::new()).collect();
        for (owner, lines) in owners.into_iter().zip(lines) {
            read[owner].push(lines);
        }
        for (task, lines) in tasks.iter().zip(read) {
            log.record(task.crops.index, task.hud, RegionSet::empty(), lines);
        }
        Ok(())
    }

    /// Reads the tasks from `tasks` until it closes or `stopped` says to
    /// stop, the frames waiting at each turn together; returns the lines
    /// in a log of a video with `half_second` frames in half a second.
    fn serve(
        &mut self,
        tasks: &Receiver<Task>,
        half_second: u32,
        stopped: &(dyn Fn() -> bool + Sync),
    ) -> Result<LineLog, String> {
        let mut log = LineLog::new(half_second);
        while let Ok(first) = tasks.recv() {
            if stopped() {
                break;
            }
            let mut batch = vec![first];
            while batch.len() < BATCH_FRAMES {
                match tasks.try_recv() {
                    Ok(task) => batch.push(task),
                    Err(_) => break,
                }
            }
            let reads: Vec<ZoneRead<'_>> = batch.iter().map(Task::as_read).collect();
            self.read(&reads, &mut log)?;
        }
        Ok(log)
    }
}

/// The zones of `kinds` (a reader's zones) that `wanted` picks and
/// `crops` holds, as indices into them.
fn zones_of(
    kinds: &[ZoneKind],
    crops: &FrameCrops,
    wanted: &dyn Fn(ZoneKind) -> bool,
) -> Vec<usize> {
    kinds
        .iter()
        .enumerate()
        .filter(|&(_, &kind)| wanted(kind) && crops.has_zone(kind))
        .map(|(i, _)| i)
        .collect()
}

/// The reader group of a zone of `kind` when an analysis reads with
/// `readers` engines: the kill feed and banner, the left panel, the screen
/// surfaces, the full frame.
fn group_of(kind: ZoneKind, readers: usize) -> usize {
    let group = match kind {
        ZoneKind::FeedAndBanner => 0,
        ZoneKind::LeftPanel => 1,
        ZoneKind::Screen(_) => 2,
        ZoneKind::FullFrame => 3,
    };
    group % readers.max(1)
}

/// OCR engines built ahead of an analysis, while the video is probed and
/// calibrated (creating the sessions takes over a second, mostly
/// DirectML compiling their graphs), each with the zone sizes it was
/// built for.
pub(crate) struct Warmed {
    runtime: OrtRuntime,
    engines: Vec<(Vec<ZoneSpec>, Result<OcrEngine, String>)>,
}

/// OCR engines being built on a thread of their own.
pub(crate) type Warming = std::thread::JoinHandle<Result<Warmed, String>>;

impl Warmed {
    /// Loads ONNX Runtime and the models of `setup` and builds one engine
    /// for each list of zone sizes in `groups`, all at the same time.
    fn build(
        setup: &OcrSetup,
        adapter: Option<Luid>,
        groups: Vec<Vec<ZoneSpec>>,
    ) -> Result<Self, String> {
        let runtime = OrtRuntime::load(&setup.ort_dylib).map_err(|e| e.to_string())?;
        let models = ModelSet::load(&setup.model_dir).map_err(|e| e.to_string())?;
        let adapter = adapter.and_then(|luid| {
            blurrd_video::sys::gpu::dxgi_adapter_index(luid)
                .map(|dxgi_index| GpuAdapter { luid, dxgi_index })
        });
        // On the CPU the engines share the cores.
        let cpu_threads = (OcrOptions::default().cpu_threads / groups.len().max(1)).max(1);
        let built: Vec<Result<OcrEngine, String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = groups
                .iter()
                .map(|zones| {
                    let options = OcrOptions {
                        ep: if setup.gpu { Ep::Auto } else { Ep::Cpu },
                        adapter,
                        zones: zones.clone(),
                        cpu_threads,
                    };
                    let (runtime, models) = (&runtime, &models);
                    scope.spawn(move || {
                        OcrEngine::new(runtime, models, &options).map_err(|e| e.to_string())
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|_| Err(String::from("building an OCR engine panicked")))
                })
                .collect()
        });
        Ok(Self {
            runtime,
            engines: groups.into_iter().zip(built).collect(),
        })
    }

    /// The engines of `warmed` with the ones missing for the zone sizes
    /// in `specs` built from `setup` (at the same time); all of them
    /// when nothing was warmed.
    fn complete(
        warmed: Option<Self>,
        setup: &OcrSetup,
        adapter: Option<Luid>,
        specs: &[Vec<ZoneSpec>],
    ) -> Result<Self, String> {
        let Some(mut warmed) = warmed else {
            return Self::build(setup, adapter, specs.to_vec());
        };
        let missing: Vec<Vec<ZoneSpec>> = specs
            .iter()
            .filter(|zones| !warmed.engines.iter().any(|(z, _)| z == *zones))
            .cloned()
            .collect();
        if !missing.is_empty() {
            tracing::debug!(
                engines = missing.len(),
                "building OCR engines for the calibrated zone sizes"
            );
            warmed
                .engines
                .extend(Self::build(setup, adapter, missing)?.engines);
        }
        Ok(warmed)
    }

    /// Takes the engine built for exactly the zone sizes `zones`.
    fn take(&mut self, zones: &[ZoneSpec]) -> Result<OcrEngine, String> {
        let i = self
            .engines
            .iter()
            .position(|(z, _)| z == zones)
            .ok_or_else(|| String::from("an OCR engine is missing"))?;
        let ocr = self.engines.swap_remove(i).1?;
        if let Some(reason) = ocr.fallback() {
            tracing::info!(%reason, "OCR runs on the CPU");
        }
        Ok(ocr)
    }
}

/// The engines a finished warm-up built: `None` when it did not finish
/// (they are built again), the reason when OCR cannot run.
fn warmed(warming: Option<Warming>) -> Result<Option<Warmed>, String> {
    match warming.map(std::thread::JoinHandle::join) {
        Some(Ok(Ok(warmed))) => Ok(Some(warmed)),
        Some(Ok(Err(reason))) => Err(reason),
        Some(Err(_)) | None => Ok(None),
    }
}

/// An empty line log of the video `info`.
fn line_log(info: &MediaInfo) -> LineLog {
    let fps = f64::from(info.fps.num) / f64::from(info.fps.den.max(1));
    LineLog::new((fps * 0.5).round().clamp(1.0, 1e6) as u32)
}

/// What the crop workers cut for the zones of `builders`, with full-frame
/// samples as `log` takes them.
fn ocr_reads<'a>(builders: impl Iterator<Item = &'a DictionaryBuilder>, log: &LineLog) -> OcrReads {
    let mut screens = [None; 4];
    for zone in builders.flat_map(DictionaryBuilder::zones) {
        if let ZoneKind::Screen(region) = zone.kind
            && let Some(i) = Region::SCREENS.iter().position(|&r| r == region)
        {
            screens[i] = Some(zone.rect);
        }
    }
    OcrReads {
        full_every: log.full_every(),
        screens,
    }
}

/// Pass 0 of an analysis whose OCR engines may still be building: the
/// analysis starts right away, and [`Opening::run`] hands each reader its
/// engine as soon as it is built (the zones due before wait in the
/// reader's queue).
pub(crate) struct Opening {
    setup: OcrSetup,
    adapter: Option<Luid>,
    warming: Option<Warming>,
    /// One per reader, with the zone sizes its engine is built for.
    builders: Vec<DictionaryBuilder>,
    specs: Vec<Vec<ZoneSpec>>,
    coverage: Coverage,
    log: LineLog,
}

impl Opening {
    /// Pass 0 for the video `info` with the HUD at `geometry`, reading the
    /// screen surfaces of `regions` and the frames of `coverage` with up
    /// to `readers` OCR engines (as [`Pass0::open`]); the engines of
    /// `warming` built for the right zone sizes are used, the others are
    /// built from `setup` once the warm-up finished.
    #[allow(
        clippy::too_many_arguments,
        reason = "what pass 0 reads and with which engines; a struct would only name them again"
    )]
    pub(crate) fn new(
        setup: &OcrSetup,
        adapter: Option<Luid>,
        geometry: &HudGeometry,
        info: &MediaInfo,
        regions: RegionSet,
        coverage: Coverage,
        readers: usize,
        warming: Option<Warming>,
    ) -> Self {
        let builders = builders(geometry, (info.width, info.height), regions, readers);
        let specs = builders.iter().map(DictionaryBuilder::zone_specs).collect();
        Self {
            setup: setup.clone(),
            adapter,
            warming,
            builders,
            specs,
            coverage,
            log: line_log(info),
        }
    }

    /// What the crop workers cut for this pass 0.
    pub(crate) fn reads(&self) -> OcrReads {
        ocr_reads(self.builders.iter(), &self.log)
    }

    /// Reads the frames from `frames` until it closes or `stopped` says
    /// the analysis stops (an error or a cancellation: the frames still
    /// queued are dropped unread), each reader on a thread of its own
    /// once its engine is built. `structure` delivers `(frame, structure)`
    /// from each of the `hud_stages` HUD stages. Returns pass 0 with what
    /// it read; fails with the reason when OCR cannot run or fails.
    pub(crate) fn run(
        self,
        frames: &Receiver<Arc<FrameCrops>>,
        structure: &Receiver<(u32, Structure)>,
        hud_stages: usize,
        stopped: &(dyn Fn() -> bool + Sync),
    ) -> Result<Pass0, String> {
        let Self {
            setup,
            adapter,
            warming,
            builders,
            specs,
            coverage,
            mut log,
        } = self;
        let mut schedule = ReadSchedule::new(coverage, log.half_second());
        log.set_hud_spacing(schedule.spacing());
        let half_second = log.half_second();
        let plans: Vec<Vec<ZoneKind>> = builders
            .iter()
            .map(|b| b.zones().iter().map(|z| z.kind).collect())
            .collect();
        let mut counted = 0u64;
        // The runtime is bound before the readers, so their sessions are
        // closed first on every path (the runtime must outlive them).
        let (runtime, served) = std::thread::scope(|scope| {
            let mut queues: Vec<Sender<Task>> = Vec::with_capacity(builders.len());
            let mut handoffs = Vec::with_capacity(builders.len());
            let mut workers = Vec::with_capacity(builders.len());
            for builder in builders {
                let (tx, rx) = bounded::<Task>(READER_QUEUE);
                let (give, take) = bounded::<Result<OcrEngine, String>>(1);
                queues.push(tx);
                handoffs.push(give);
                workers.push(scope.spawn(move || {
                    let ocr = take
                        .recv()
                        .unwrap_or_else(|_| Err(String::from("the OCR engines were not built")))?;
                    let mut reader = Reader { ocr, builder };
                    let part = reader.serve(&rx, half_second, stopped)?;
                    Ok((reader, part))
                }));
            }
            // The engines are finished on a thread of their own while the
            // readers' queues fill.
            let specs = &specs;
            let engines = scope.spawn(move || {
                match warmed(warming).and_then(|w| Warmed::complete(w, &setup, adapter, specs)) {
                    Ok(mut warmed) => {
                        for (give, zones) in handoffs.iter().zip(specs) {
                            let _ = give.send(warmed.take(zones));
                        }
                        Some(warmed.runtime)
                    }
                    Err(reason) => {
                        for give in &handoffs {
                            let _ = give.send(Err(reason.clone()));
                        }
                        None
                    }
                }
            });
            let kinds: Vec<ZoneKind> = plans.iter().flatten().copied().collect();
            let mut signals = Signals::new(hud_stages);
            for crops in frames {
                let index = crops.index;
                let Some(seen) = signals.wait(index, structure, stopped) else {
                    break;
                };
                let screens = Region::SCREENS.map(|region| {
                    let kind = ZoneKind::Screen(region);
                    kinds.contains(&kind) && crops.has_zone(kind)
                });
                let due = schedule.due(index, &seen, screens);
                let Some(read) = dispatch(&plans, &queues, &crops, due) else {
                    break;
                };
                counted += u64::from(read);
                if !seen.regions.is_empty() {
                    log.record(index, HudRead::NONE, seen.regions, Vec::new());
                }
            }
            drop(queues);
            let runtime = engines.join().ok().flatten();
            let served: Vec<Result<(Reader, LineLog), String>> = workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .unwrap_or_else(|_| Err(String::from("an OCR worker panicked")))
                })
                .collect();
            (runtime, served)
        });
        let served = served.into_iter().collect::<Result<Vec<_>, String>>()?;
        let runtime = runtime.ok_or_else(|| String::from("the OCR runtime is missing"))?;
        let mut readers = Vec::with_capacity(served.len());
        for (reader, part) in served {
            log.absorb(part);
            readers.push(reader);
        }
        Ok(Pass0 {
            _runtime: runtime,
            readers,
            coverage,
            log,
            frames: counted,
        })
    }
}

/// The builders of the zones pass 0 reads in a `width` × `height` video
/// with the HUD at `geometry` and the screen surfaces of `regions`, one
/// per reader of `readers` (clamped to 1..=[`Pass0::MAX_READERS`]).
fn builders(
    geometry: &HudGeometry,
    (width, height): (u32, u32),
    regions: RegionSet,
    readers: usize,
) -> Vec<DictionaryBuilder> {
    let readers = readers.clamp(1, Pass0::MAX_READERS);
    DictionaryBuilder::verifying(geometry, width, height, regions)
        .split(|kind| group_of(kind, readers))
}

impl Pass0 {
    /// The most OCR engines an analysis reads with ([`group_of`]).
    pub(crate) const MAX_READERS: usize = 4;

    /// Starts building the OCR engines of `setup` for a `width` ×
    /// `height` video and the screen surfaces of `regions` with `readers`
    /// engines, guessing the HUD geometry from the frame size; the
    /// calibrated geometry almost always gives the same zone sizes, and
    /// [`Self::open`] builds the others again.
    pub(crate) fn warm_up(
        setup: &OcrSetup,
        adapter: Option<Luid>,
        (width, height): (u32, u32),
        regions: RegionSet,
        readers: usize,
    ) -> Result<Warming, String> {
        let geometry = HudGeometry::for_layout(width, height, HudLayout::Auto);
        let groups = builders(&geometry, (width, height), regions, readers)
            .iter()
            .map(DictionaryBuilder::zone_specs)
            .collect();
        let setup = setup.clone();
        std::thread::Builder::new()
            .name(String::from("blurrd-ocr-warm-up"))
            .spawn(move || Warmed::build(&setup, adapter, groups))
            .map_err(|e| format!("the OCR engines could not be built: {e}"))
    }

    /// Opens pass 0 for the video `info` with the HUD at `geometry`,
    /// reading the screen surfaces of `regions` and the frames of
    /// `coverage` with up to `readers` OCR engines (at most
    /// [`Self::MAX_READERS`]; one reads every zone). Engines of `warmed`
    /// built for the right zone sizes are used, the others are built from
    /// `setup` (at the same time). Fails with the reason when OCR cannot
    /// run (the analysis then goes on without it).
    #[allow(
        clippy::too_many_arguments,
        reason = "what pass 0 reads and with which engines; a struct would only name them again"
    )]
    pub(crate) fn open(
        setup: &OcrSetup,
        adapter: Option<Luid>,
        geometry: &HudGeometry,
        info: &MediaInfo,
        regions: RegionSet,
        coverage: Coverage,
        readers: usize,
        warmed: Option<Warmed>,
    ) -> Result<Self, String> {
        let builders = builders(geometry, (info.width, info.height), regions, readers);
        let specs: Vec<Vec<ZoneSpec>> =
            builders.iter().map(DictionaryBuilder::zone_specs).collect();
        let mut warmed = Warmed::complete(warmed, setup, adapter, &specs)?;
        let mut readers = Vec::with_capacity(builders.len());
        for (builder, zones) in builders.into_iter().zip(&specs) {
            readers.push(Reader {
                ocr: warmed.take(zones)?,
                builder,
            });
        }
        Ok(Self {
            _runtime: warmed.runtime,
            readers,
            coverage,
            log: line_log(info),
            frames: 0,
        })
    }

    /// What the crop workers cut for this pass 0.
    pub(crate) fn reads(&self) -> OcrReads {
        ocr_reads(self.readers.iter().map(|r| &r.builder), &self.log)
    }

    /// The counters so far.
    pub(crate) fn stats(&self) -> Pass0Stats {
        let mut total = Pass0Stats {
            frames: self.frames,
            ..Pass0Stats::default()
        };
        for reader in &self.readers {
            let stats = reader.builder.stats();
            total.zone_crops += stats.zone_crops;
            total.zone_crops_read += stats.zone_crops_read;
            total.lines += stats.lines;
            total.name_reads += stats.name_reads;
            total.ocr_seconds += stats.ocr_seconds;
        }
        total
    }

    /// The frames it read.
    pub(crate) fn coverage(&self) -> Coverage {
        self.coverage
    }

    /// What the verification reads.
    pub(crate) fn parts(&mut self) -> Pass0Parts<'_> {
        let reader = &mut self.readers[0];
        Pass0Parts {
            ocr: &mut reader.ocr,
            whitelist: reader.builder.whitelist(),
            log: &self.log,
        }
    }

    /// Reads the zones of one frame's `crops`: the HUD zones when `hud`
    /// is set, the screen zones and the full frame when the crops hold
    /// them; the frame showed the HUD structure of `structure`. Returns
    /// `false` when `stopped` said to stop first; fails with the reason
    /// when OCR fails.
    pub(crate) fn read(
        &mut self,
        crops: &FrameCrops,
        hud: bool,
        structure: RegionSet,
        stopped: &dyn Fn() -> bool,
    ) -> Result<bool, String> {
        let wanted = |kind: ZoneKind| hud || !HudRead::of(kind).any();
        let hud_read = if hud { HudRead::BOTH } else { HudRead::NONE };
        let mut read = false;
        for reader in &mut self.readers {
            if stopped() {
                return Ok(false);
            }
            let zones = zones_of(&reader.kinds(), crops, &wanted);
            if zones.is_empty() {
                continue;
            }
            read = true;
            let task = ZoneRead {
                crops,
                zones: &zones,
                hud: hud_read,
            };
            reader.read(&[task], &mut self.log)?;
        }
        self.frames += u64::from(hud || read);
        if hud || !structure.is_empty() {
            self.log
                .record(crops.index, hud_read, structure, Vec::new());
        }
        Ok(true)
    }
}

/// The structure signals of the HUD stages, collected per frame.
struct Signals {
    /// Per frame: the stages that sent theirs, and what they saw.
    pending: BTreeMap<u32, (usize, Structure)>,
    stages: usize,
}

impl Signals {
    fn new(stages: usize) -> Self {
        Self {
            pending: BTreeMap::new(),
            stages,
        }
    }

    /// What the stages saw on `frame`, once each of them sent it from
    /// `structure`; `None` when `stopped` says to stop or the stages
    /// stopped (the analysis is ending early).
    fn wait(
        &mut self,
        frame: u32,
        structure: &Receiver<(u32, Structure)>,
        stopped: &dyn Fn() -> bool,
    ) -> Option<Structure> {
        while self.pending.get(&frame).is_none_or(|s| s.0 < self.stages) {
            if stopped() {
                return None;
            }
            let (at, seen) = match structure.recv_timeout(POLL) {
                Ok(signal) => signal,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return None,
            };
            let entry = self.pending.entry(at).or_default();
            entry.0 += 1;
            entry.1.merge(&seen);
        }
        Some(self.pending.remove(&frame).map(|s| s.1).unwrap_or_default())
    }
}

/// Queues the zones of `crops` that are `due` for the readers whose zones
/// `plans` lists, on their `queues`. Returns whether any zone was queued;
/// `None` when a reader failed (it dropped its queue, and its error ends
/// pass 0).
fn dispatch(
    plans: &[Vec<ZoneKind>],
    queues: &[Sender<Task>],
    crops: &Arc<FrameCrops>,
    due: Due,
) -> Option<bool> {
    let wanted = |kind: ZoneKind| match kind {
        ZoneKind::FeedAndBanner => due.hud.feed,
        ZoneKind::LeftPanel => due.hud.left,
        ZoneKind::Screen(region) => Region::SCREENS
            .iter()
            .position(|&r| r == region)
            .is_some_and(|i| due.screens[i]),
        ZoneKind::FullFrame => true,
    };
    let mut read = false;
    for (plan, queue) in plans.iter().zip(queues) {
        let zones = zones_of(plan, crops, &wanted);
        if zones.is_empty() {
            continue;
        }
        read = true;
        let hud = zones
            .iter()
            .fold(HudRead::NONE, |hud, &i| hud.or(HudRead::of(plan[i])));
        let task = Task {
            crops: Arc::clone(crops),
            zones,
            hud,
        };
        queue.send(task).ok()?;
    }
    Some(read)
}

/// The HUD zone that holds the names of `region` (one of
/// [`LAYOUT_REGIONS`]).
fn zone_of(region: Region) -> ZoneKind {
    match region {
        Region::KillFeed | Region::Banner => ZoneKind::FeedAndBanner,
        _ => ZoneKind::LeftPanel,
    }
}

/// Which zones pass 0 reads on one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Due {
    /// The HUD zones.
    hud: HudRead,
    /// The name zone of each screen surface, in [`Region::SCREENS`] order.
    screens: [bool; 4],
}

/// When pass 0 reads the zones during an analysis.
///
/// Each HUD zone is read when the layout of the structure of its regions
/// changes ([`Structure::layout`]: the kill feed and banner, or the voice
/// and text chat), again [`ReadSchedule::settle`] frames after the
/// last change (fades and slides have ended and the text reads best), and
/// whenever a second passed since it was last read. A change read
/// follows the previous read by at least [`ReadSchedule::gap`] frames, so a
/// flickering detection does not read every frame; the settle read still
/// comes. A screen surface's name zone follows the same rules with its
/// gate (a candidate frame) as its layout and is read only while the gate
/// is on. With [`Coverage::EveryFrame`] every zone is read on every frame
/// it can be.
#[derive(Debug, Clone)]
struct ReadSchedule {
    /// Read every zone on every frame it can be ([`Coverage::EveryFrame`]).
    every: bool,
    /// Frames between two reads of a zone at most (a second).
    cadence: u32,
    /// Frames from a change to its settle read.
    settle: u32,
    /// Frames from one read to a change read at least.
    gap: u32,
    /// The kill feed and banner zone, then the left panel.
    hud: [ZoneClock; 2],
    screens: [ZoneClock; 4],
}

/// The read state of one zone.
#[derive(Debug, Clone, Copy, Default)]
struct ZoneClock {
    /// The layout on the previous frame (`None` before the first).
    layout: Option<u64>,
    /// The frame it was last read on.
    last: Option<u32>,
    /// The frame of its pending settle read.
    settle: Option<u32>,
}

impl ZoneClock {
    /// Whether the zone is read on `frame`, where its layout is `layout`.
    fn due(&mut self, frame: u32, layout: u64, schedule: &ReadSchedule) -> bool {
        let since = self.last.map(|last| frame.saturating_sub(last));
        let mut due = since.is_none_or(|since| since >= schedule.cadence);
        let previous = self.layout.replace(layout);
        if previous.is_some_and(|previous| previous != layout) {
            self.settle = Some(frame + schedule.settle);
            due |= since.is_none_or(|since| since >= schedule.gap);
        } else if self.settle.is_some_and(|settle| frame >= settle) {
            self.settle = None;
            due = true;
        }
        if due {
            self.last = Some(frame);
        }
        due
    }
}

impl ReadSchedule {
    /// The schedule of `coverage` for a video with `half_second` frames in
    /// half a second.
    fn new(coverage: Coverage, half_second: u32) -> Self {
        let half_second = half_second.max(1);
        Self {
            every: coverage == Coverage::EveryFrame,
            cadence: 2 * half_second,
            // A fifth of a second: kill-feed rows slide and fade in about
            // ten frames at 60 fps.
            settle: (half_second * 2 / 5).max(1),
            gap: (half_second / 10).max(1),
            hud: [ZoneClock::default(); 2],
            screens: [ZoneClock::default(); 4],
        }
    }

    /// The most frames between two HUD reads of one name on screen: what
    /// joins the exposures of one name into one run.
    fn spacing(&self) -> u32 {
        if self.every {
            DictionaryBuilder::CADENCE
        } else {
            self.cadence
        }
    }

    /// The zones read on `frame`, where the HUD stages saw `seen` and the
    /// screen surfaces in [`Region::SCREENS`] order are candidates as in
    /// `screens`.
    fn due(&mut self, frame: u32, seen: &Structure, screens: [bool; 4]) -> Due {
        let zones = [ZoneKind::FeedAndBanner, ZoneKind::LeftPanel];
        let mut hud = [false; 2];
        for (i, zone) in zones.into_iter().enumerate() {
            // The fingerprints of the regions whose names the zone holds.
            let layout = LAYOUT_REGIONS
                .iter()
                .zip(seen.layout)
                .filter(|&(&region, _)| zone_of(region) == zone)
                .fold(0u64, |key, (_, part)| key.rotate_left(17) ^ part);
            let mut clock = self.hud[i];
            hud[i] = clock.due(frame, layout, self) || self.every;
            self.hud[i] = clock;
        }
        let mut due = Due {
            hud: HudRead {
                feed: hud[0],
                left: hud[1],
            },
            screens: [false; 4],
        };
        for (i, &candidate) in screens.iter().enumerate() {
            let mut clock = self.screens[i];
            // Off frames have nothing to read; the gate opening is the
            // change that reads first.
            let on = clock.due(frame, u64::from(candidate), self);
            due.screens[i] = candidate && (on || self.every);
            if !candidate {
                clock.settle = None;
                clock.last = None;
            }
            self.screens[i] = clock;
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{Coverage, RegionSet};

    use super::{Due, ReadSchedule, Structure};

    /// What the HUD stages saw: the kill feed with layout `feed` and the
    /// chat with layout `chat`.
    fn seen(feed: u64, chat: u64) -> Structure {
        Structure {
            regions: RegionSet::empty(),
            layout: [feed, 0, 0, chat],
        }
    }

    /// The frames of `0..frames` on which `pick` says a zone is read.
    fn reads(
        schedule: &mut ReadSchedule,
        frames: u32,
        at: impl Fn(u32) -> (Structure, [bool; 4]),
        pick: impl Fn(&Due) -> bool,
    ) -> Vec<u32> {
        (0..frames)
            .filter(|&f| {
                let (structure, screens) = at(f);
                pick(&schedule.due(f, &structure, screens))
            })
            .collect()
    }

    #[test]
    fn reads_a_still_zone_once_a_second() {
        // 60 fps: half a second is 30 frames, the cadence 60.
        let mut schedule = ReadSchedule::new(Coverage::Structure, 30);
        let feed = reads(
            &mut schedule,
            200,
            |_| (seen(7, 9), [false; 4]),
            |d| d.hud.feed,
        );
        assert_eq!(feed, [0, 60, 120, 180]);
        assert_eq!(schedule.spacing(), 60);
    }

    #[test]
    fn reads_a_change_when_it_comes_and_when_it_settled() {
        let mut schedule = ReadSchedule::new(Coverage::Structure, 30);
        // The kill feed changes at frame 30 and flickers on 31 and 32 (a
        // row sliding in): one read at the change, one when it settled
        // (12 frames after the last change), then the cadence again.
        let at = |f: u32| {
            let feed = if f < 30 {
                1
            } else if f == 31 {
                3
            } else {
                2
            };
            (seen(feed, 5), [false; 4])
        };
        let feed = reads(&mut schedule, 120, at, |d| d.hud.feed);
        assert_eq!(feed, [0, 30, 44, 104]);
    }

    #[test]
    fn reads_each_hud_zone_at_its_own_changes() {
        let mut schedule = ReadSchedule::new(Coverage::Structure, 30);
        let at = |f: u32| (seen(1, if f < 20 { 1 } else { 2 }), [false; 4]);
        let mut feed = Vec::new();
        let mut left = Vec::new();
        for f in 0..40 {
            let (structure, screens) = at(f);
            let due = schedule.due(f, &structure, screens);
            if due.hud.feed {
                feed.push(f);
            }
            if due.hud.left {
                left.push(f);
            }
        }
        assert_eq!(feed, [0]);
        assert_eq!(left, [0, 20, 32]);
    }

    #[test]
    fn reads_a_screen_only_while_its_gate_is_on() {
        let mut schedule = ReadSchedule::new(Coverage::Structure, 30);
        // The scoreboard is a candidate on frames 10..100 and 150..160.
        let at = |f: u32| {
            let on = (10..100).contains(&f) || (150..160).contains(&f);
            (seen(1, 1), [on, false, false, false])
        };
        let board = reads(&mut schedule, 200, at, |d| d.screens[0]);
        assert_eq!(board, [10, 22, 82, 150]);
    }

    #[test]
    fn every_frame_coverage_reads_every_frame() {
        let mut schedule = ReadSchedule::new(Coverage::EveryFrame, 30);
        let both = reads(
            &mut schedule,
            10,
            |_| (seen(1, 1), [true, false, false, false]),
            |d| d.hud.feed && d.hud.left && d.screens[0],
        );
        assert_eq!(both, (0..10).collect::<Vec<_>>());
        assert_eq!(schedule.spacing(), crate::DictionaryBuilder::CADENCE);
    }
}
