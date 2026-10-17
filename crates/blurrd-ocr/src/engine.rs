//! The OCR engine: ONNX Runtime sessions with fixed shapes on DirectML or
//! the CPU.
//!
//! DirectML compiles a graph for the exact input shape when a session is
//! created; a new shape at run time costs a recompilation of tens to
//! hundreds of milliseconds. So every session here has a fully static
//! input: one detection session per protected zone (its own input size)
//! and one recognition session per width bucket with a fixed batch. Every
//! session runs once at creation (warm-up), and the engine only ever builds
//! tensors of those shapes. The DirectML rules of the architecture plan
//! (5.5) apply: memory pattern off, sequential execution, one engine (and
//! so one set of sessions) per worker, the adapter chosen by LUID.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use blurrd_core::{Luid, OcrProvider, RectI};
use ort::ep::{self, ExecutionProviderDispatch};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::{TensorRef, ValueType};

use crate::det::{self, TextMap};
use crate::error::OcrError;
use crate::image::RgbView;
use crate::models::ModelSet;
use crate::rec::{self, REC_HEIGHT, Recognition, WIDTH_BUCKETS};
use crate::runtime::OrtRuntime;

/// Recognition batch per width bucket on a GPU. Wider inputs produce larger
/// outputs (steps × classes per item), so their batches are smaller.
const GPU_REC_BATCH: [u32; 3] = [16, 8, 4];
/// Recognition batch on the CPU, where a batch brings little but padded
/// items cost full price.
const CPU_REC_BATCH: [u32; 3] = [1, 1, 1];

/// The execution provider to run OCR on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ep {
    /// DirectML when it works on this machine, otherwise the CPU (the
    /// reason is kept in [`OcrEngine::fallback`]).
    #[default]
    Auto,
    /// DirectML only; fails if it cannot be used.
    DirectMl,
    /// The CPU only.
    Cpu,
}

/// A GPU picked by its LUID for DirectML.
///
/// DirectML addresses adapters by their position in DXGI's
/// `EnumAdapters1` order, which changes with drivers, docking and power
/// settings. The caller therefore resolves the LUID to that position right
/// before creating the engine (`blurrd_video::sys::gpu::dxgi_adapter_index`)
/// and never stores the position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuAdapter {
    /// The adapter's LUID (what settings and capabilities store).
    pub luid: Luid,
    /// Its current position in DXGI's `EnumAdapters1` order.
    pub dxgi_index: u32,
}

/// Identifies a protected zone, which has its own detection input size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneId(pub u16);

impl fmt::Display for ZoneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "zone {}", self.0)
    }
}

/// A protected zone and the fixed size its crops are detected at.
///
/// Crops are scaled to fit the content size with their natural aspect
/// ratio ([`RgbView::pixel_aspect`] undone, so a stretched HUD reaches the
/// model at the same text size as a native one) and placed into the
/// top-left of the input; the input is the content size rounded up to
/// multiples of 32 (`DBNet` downsamples by 32).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneSpec {
    /// The zone.
    pub id: ZoneId,
    /// Width crops are scaled to.
    pub content_width: u32,
    /// Height crops are scaled to.
    pub content_height: u32,
    /// Width of the detection input, a multiple of 32.
    pub width: u32,
    /// Height of the detection input, a multiple of 32.
    pub height: u32,
}

impl ZoneSpec {
    /// The spec for crops whose nominal size is `crop_w` × `crop_h` pixels,
    /// detected at `scale` input pixels per crop pixel. Crops of the
    /// nominal size at scale 1 reach the model unresampled.
    pub fn for_crop(id: ZoneId, crop_w: u32, crop_h: u32, scale: f32) -> Self {
        let content = |v: u32| ((v as f32 * scale).round() as u32).max(1);
        let round = |v: u32| v.max(32).div_ceil(32) * 32;
        let (content_width, content_height) = (content(crop_w), content(crop_h));
        Self {
            id,
            content_width,
            content_height,
            width: round(content_width),
            height: round(content_height),
        }
    }
}

/// How to build an [`OcrEngine`].
#[derive(Debug, Clone, PartialEq)]
pub struct OcrOptions {
    /// Execution provider.
    pub ep: Ep,
    /// The GPU for DirectML; `None` lets DirectML pick the
    /// high-performance GPU.
    pub adapter: Option<GpuAdapter>,
    /// The protected zones and their detection sizes.
    pub zones: Vec<ZoneSpec>,
    /// Threads for the CPU execution provider (ignored on DirectML).
    pub cpu_threads: usize,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            ep: Ep::Auto,
            adapter: None,
            zones: Vec::new(),
            cpu_threads: 4,
        }
    }
}

/// A recognised text line with its position: one entry of
/// [`OcrEngine::read`].
#[derive(Debug, Clone, PartialEq)]
pub struct OcrRead {
    /// Where the text is, in pixels of the zone image.
    pub rect: RectI,
    /// Detection score of the box.
    pub det_score: f32,
    /// The recognised text and its confidence.
    pub rec: Recognition,
}

/// Run-time statistics of one session (one fixed shape).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ShapeStats {
    /// Time to create the session, in milliseconds. DirectML compiles the
    /// graph for the fixed shape here.
    pub create_ms: f32,
    /// Duration of the warm-up run after creation, in milliseconds.
    pub warmup_ms: f32,
    /// Runs after the warm-up.
    pub runs: u64,
    /// Total time of those runs, in milliseconds.
    pub total_ms: f64,
    /// Slowest of those runs, in milliseconds.
    pub max_ms: f32,
    /// Runs slower than [`ShapeStats::slow_threshold_ms`]; each one is
    /// logged as a warning.
    ///
    /// This is a stall detector, not proof that no recompilation happened:
    /// a recompilation much cheaper than the session's creation stays
    /// below the threshold. The guarantee that the graph is never
    /// recompiled rests on construction instead: every input dimension is
    /// pinned when the session is built, the built session's input shape
    /// is checked against the requested one, and every tensor is built
    /// with exactly that shape.
    pub slow_runs: u64,
    /// A run slower than this counts as a stall: on DirectML the session's
    /// creation plus its warm-up, which a full graph compilation repeats.
    /// The CPU provider compiles nothing per shape, so there it is
    /// infinite and slow runs (a loaded CPU) are not counted.
    pub slow_threshold_ms: f32,
}

impl ShapeStats {
    /// Adds a run of `ms` milliseconds; returns whether it counts as a
    /// stall ([`ShapeStats::slow_threshold_ms`]).
    fn record(&mut self, ms: f32) -> bool {
        self.runs += 1;
        self.total_ms += f64::from(ms);
        self.max_ms = self.max_ms.max(ms);
        let slow = ms > self.slow_threshold_ms;
        self.slow_runs += u64::from(slow);
        slow
    }

    /// Whether the stalls stay within what a GPU shared with other work
    /// causes: at most one slow run per [`ShapeStats::STALL_RUNS`] runs,
    /// at least one allowed.
    ///
    /// Slow runs are a load metric, not evidence of a recompilation (see
    /// [`ShapeStats::slow_runs`]), so an exact zero would fail whenever
    /// another process holds the GPU for a moment. A shape compiled again
    /// on every run would make every run slow, far above this rate.
    pub fn stalls_tolerable(&self) -> bool {
        self.slow_runs <= (self.runs / Self::STALL_RUNS).max(1)
    }

    /// Runs per tolerated slow run in [`ShapeStats::stalls_tolerable`]
    /// (0.1 %).
    pub const STALL_RUNS: u64 = 1000;

    /// Mean duration of the runs after warm-up, in milliseconds.
    pub fn mean_ms(&self) -> f32 {
        if self.runs == 0 {
            0.0
        } else {
            (self.total_ms / self.runs as f64) as f32
        }
    }
}

/// Where the sessions of one build attempt run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// The CPU execution provider.
    Cpu,
    /// DirectML on this adapter, or on DirectML's high-performance choice.
    DirectMl(Option<GpuAdapter>),
}

/// A session with a fixed input shape and its statistics.
struct FixedSession {
    session: Session,
    shape: [usize; 4],
    stats: ShapeStats,
}

impl fmt::Debug for FixedSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FixedSession")
            .field("shape", &self.shape)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl FixedSession {
    /// Runs the session on `input` (exactly the session's shape) and hands
    /// the first output to `read`.
    fn run<T>(
        &mut self,
        input: &[f32],
        action: &'static str,
        read: impl FnOnce(&[i64], &[f32]) -> Result<T, OcrError>,
    ) -> Result<T, OcrError> {
        let started = Instant::now();
        let tensor =
            TensorRef::from_array_view((self.shape, input)).map_err(OcrError::session(action))?;
        let outputs = self
            .session
            .run(ort::inputs![tensor])
            .map_err(OcrError::session(action))?;
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(OcrError::session(action))?;
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        if self.stats.record(ms) {
            tracing::warn!(
                shape = ?self.shape,
                ms,
                threshold_ms = self.stats.slow_threshold_ms,
                "an OCR run took longer than creating and warming up its session \
                 (recompilation or a stalled GPU)"
            );
        }
        read(shape, data)
    }
}

/// PP-OCR text detection and recognition on one worker.
///
/// Methods take `&mut self`: a session must never run twice at the same
/// time (DirectML rule), so every worker owns its own engine.
#[derive(Debug)]
pub struct OcrEngine {
    provider: OcrProvider,
    fallback: Option<String>,
    det: BTreeMap<ZoneId, (ZoneSpec, FixedSession)>,
    rec: Vec<FixedSession>,
    rec_batch: [u32; 3],
    classes: usize,
    charset: Arc<[String]>,
}

impl OcrEngine {
    /// Creates the sessions for every zone and width bucket and warms each
    /// of them up once.
    ///
    /// With [`Ep::Auto`], a DirectML failure (no DirectML in the runtime,
    /// no usable GPU, a session that fails to build) falls back to the CPU;
    /// [`OcrEngine::fallback`] says why.
    ///
    /// # Errors
    ///
    /// [`OcrError::ProviderUnavailable`] for [`Ep::DirectMl`] without
    /// DirectML, [`OcrError::Session`] or [`OcrError::Signature`] when the
    /// models cannot run, [`OcrError::InvalidInput`] for zone sizes that
    /// are not positive multiples of 32.
    pub fn new(rt: &OrtRuntime, models: &ModelSet, options: &OcrOptions) -> Result<Self, OcrError> {
        for zone in &options.zones {
            let fits = zone.content_width >= 1
                && zone.content_height >= 1
                && zone.content_width <= zone.width
                && zone.content_height <= zone.height;
            if zone.width == 0
                || zone.height == 0
                || !zone.width.is_multiple_of(32)
                || !zone.height.is_multiple_of(32)
                || !fits
            {
                return Err(OcrError::InvalidInput(format!(
                    "{} detection size {}x{} (content {}x{}) is not a positive multiple of 32 \
                     holding its content",
                    zone.id, zone.width, zone.height, zone.content_width, zone.content_height
                )));
            }
        }
        match options.ep {
            Ep::Cpu => Self::build(models, options, Target::Cpu),
            Ep::DirectMl => {
                if !rt.has_directml() {
                    return Err(OcrError::ProviderUnavailable {
                        provider: "DirectML",
                        reason: "the loaded ONNX Runtime has no DirectML support".to_owned(),
                    });
                }
                Self::build(models, options, Target::DirectMl(options.adapter))
            }
            Ep::Auto => {
                let attempt = if rt.has_directml() {
                    Self::build(models, options, Target::DirectMl(options.adapter))
                } else {
                    Err(OcrError::ProviderUnavailable {
                        provider: "DirectML",
                        reason: "the loaded ONNX Runtime has no DirectML support".to_owned(),
                    })
                };
                match attempt {
                    Ok(engine) => Ok(engine),
                    Err(error) => {
                        tracing::warn!(%error, "DirectML OCR failed; falling back to the CPU");
                        let mut engine = Self::build(models, options, Target::Cpu)?;
                        engine.fallback = Some(error.to_string());
                        Ok(engine)
                    }
                }
            }
        }
    }

    /// Builds and warms up every session on `target`.
    fn build(models: &ModelSet, options: &OcrOptions, target: Target) -> Result<Self, OcrError> {
        let started = Instant::now();
        let provider = match target {
            Target::Cpu => OcrProvider::Cpu,
            Target::DirectMl(_) => OcrProvider::DirectMl,
        };
        let session = |model: &[u8], free: &[Option<String>], shape: [usize; 4]| {
            create_session(model, &overrides(free, shape)?, target, options.cpu_threads)
        };
        let det_free = free_dims(models.det())?;
        let mut det = BTreeMap::new();
        for zone in &options.zones {
            let shape = [1, 3, zone.height as usize, zone.width as usize];
            let fixed = session(models.det(), &det_free, shape)?;
            det.insert(
                zone.id,
                (
                    *zone,
                    warm_up(fixed, shape, target, "warming up detection")?,
                ),
            );
        }
        let rec_batch = match target {
            Target::Cpu => CPU_REC_BATCH,
            Target::DirectMl(_) => GPU_REC_BATCH,
        };
        let rec_free = free_dims(models.rec())?;
        let mut rec_sessions = Vec::with_capacity(WIDTH_BUCKETS.len());
        let mut classes = 0;
        for (bucket, &width) in WIDTH_BUCKETS.iter().enumerate() {
            let shape = [
                rec_batch[bucket] as usize,
                3,
                REC_HEIGHT as usize,
                width as usize,
            ];
            let fixed = session(models.rec(), &rec_free, shape)?;
            let fixed = warm_up(fixed, shape, target, "warming up recognition")?;
            classes = output_classes(&fixed)?;
            rec_sessions.push(fixed);
        }
        let charset = models.charset().clone();
        if classes != charset.len() + 1 {
            return Err(OcrError::Signature(format!(
                "the recognition model has {classes} classes but the character list has {} \
                 entries (plus the blank)",
                charset.len()
            )));
        }
        tracing::info!(
            ?provider,
            zones = det.len(),
            ms = started.elapsed().as_millis() as u64,
            "OCR sessions ready"
        );
        Ok(Self {
            provider,
            fallback: None,
            det,
            rec: rec_sessions,
            rec_batch,
            classes,
            charset,
        })
    }

    /// The execution provider the sessions run on.
    pub fn active_provider(&self) -> OcrProvider {
        self.provider
    }

    /// Why [`Ep::Auto`] fell back to the CPU, if it did.
    pub fn fallback(&self) -> Option<&str> {
        self.fallback.as_deref()
    }

    /// Detects text lines in a crop of `zone`. The crop is scaled into the
    /// zone's fixed detection size with its natural aspect ratio (see
    /// [`RgbView::pixel_aspect`]); boxes are in pixels of the crop.
    ///
    /// # Errors
    ///
    /// [`OcrError::InvalidInput`] for an unknown zone, [`OcrError::Session`]
    /// when the model fails.
    pub fn detect(&mut self, zone: ZoneId, image: RgbView<'_>) -> Result<TextMap, OcrError> {
        let (spec, session) = self
            .det
            .get_mut(&zone)
            .ok_or_else(|| OcrError::InvalidInput(format!("{zone} has no detection session")))?;
        let mut input = vec![0.0f32; 3 * spec.width as usize * spec.height as usize];
        let placement = det::preprocess(
            image,
            (spec.content_width, spec.content_height),
            (spec.width, spec.height),
            &mut input,
        );
        let input_w = spec.width;
        session.run(&input, "running text detection", |shape, data| {
            if shape.len() != 4
                || shape[1] != 1
                || data.len() != (spec.width * spec.height) as usize
            {
                return Err(OcrError::Signature(format!(
                    "detection output shape {shape:?} for input {}x{}",
                    spec.width, spec.height
                )));
            }
            Ok(det::postprocess(data, input_w, placement, image.bounds()))
        })
    }

    /// Recognises the text in each crop (one line of text per crop), in
    /// order.
    ///
    /// # Errors
    ///
    /// [`OcrError::Session`] or [`OcrError::Signature`] when the model
    /// fails.
    pub fn recognize(&mut self, crops: &[RgbView<'_>]) -> Result<Vec<Recognition>, OcrError> {
        let mut results: Vec<Option<Recognition>> = vec![None; crops.len()];
        let mut by_bucket: [Vec<usize>; 3] = Default::default();
        for (i, crop) in crops.iter().enumerate() {
            let scaled = rec::scaled_width(*crop);
            by_bucket[rec::bucket_for(scaled)].push(i);
        }
        for (bucket, indices) in by_bucket.iter().enumerate() {
            let width = WIDTH_BUCKETS[bucket];
            let batch = self.rec_batch[bucket] as usize;
            let item = (3 * REC_HEIGHT * width) as usize;
            let mut input = vec![0.0f32; batch * item];
            for chunk in indices.chunks(batch) {
                let mut columns = Vec::with_capacity(chunk.len());
                for (slot, &i) in chunk.iter().enumerate() {
                    columns.push(rec::preprocess(
                        crops[i],
                        width,
                        &mut input[slot * item..][..item],
                    ));
                }
                // Clear the slots a previous, fuller chunk left behind.
                for slot in chunk.len()..batch {
                    input[slot * item..][..item].fill(0.0);
                }
                let (classes, charset) = (self.classes, &self.charset);
                let decoded =
                    self.rec[bucket].run(&input, "running text recognition", |shape, data| {
                        rec::decode_batch(shape, data, &columns, width, batch, classes, charset)
                    })?;
                for (&i, read) in chunk.iter().zip(decoded) {
                    results[i] = Some(read);
                }
            }
        }
        Ok(results.into_iter().map(Option::unwrap_or_default).collect())
    }

    /// Detects the text lines of a zone crop and recognises each of them.
    ///
    /// # Errors
    ///
    /// As [`OcrEngine::detect`] and [`OcrEngine::recognize`].
    pub fn read(&mut self, zone: ZoneId, image: RgbView<'_>) -> Result<Vec<OcrRead>, OcrError> {
        let map = self.detect(zone, image)?;
        // Detection clips every box to the image, so each box has a crop;
        // a box without one is skipped together with its crop, which keeps
        // boxes and reads aligned.
        let (boxes, crops): (Vec<&det::TextBox>, Vec<RgbView<'_>>) = map
            .boxes
            .iter()
            .filter_map(|b| image.crop(b.rect).map(|crop| (b, crop)))
            .unzip();
        let reads = self.recognize(&crops)?;
        Ok(boxes
            .into_iter()
            .zip(reads)
            .map(|(b, rec)| OcrRead {
                rect: b.rect,
                det_score: b.score,
                rec,
            })
            .collect())
    }

    /// Statistics of every session: detection per zone, then recognition
    /// per width bucket, each with its input shape.
    pub fn stats(&self) -> Vec<([usize; 4], ShapeStats)> {
        self.det
            .values()
            .map(|(_, s)| (s.shape, s.stats))
            .chain(self.rec.iter().map(|s| (s.shape, s.stats)))
            .collect()
    }
}

/// The free dimensions of a model's input: for each position, the name of
/// the dimension if it is dynamic (`Some("")` for a dynamic dimension
/// without a name), `None` if it is fixed.
///
/// Read from a CPU session that only loads the graph (no optimisation),
/// because exporters name the dimensions differently (paddle2onnx numbers
/// them in the order it meets them, so the same position has different
/// names in different models).
fn free_dims(model: &[u8]) -> Result<Vec<Option<String>>, OcrError> {
    const ACTION: &str = "reading the model input";
    let session = Session::builder()
        .map_err(OcrError::session(ACTION))?
        .with_optimization_level(GraphOptimizationLevel::Disable)
        .map_err(OcrError::session(ACTION))?
        .with_intra_threads(1)
        .map_err(OcrError::session(ACTION))?
        .commit_from_memory(model)
        .map_err(OcrError::session(ACTION))?;
    let input = session
        .inputs()
        .first()
        .ok_or_else(|| OcrError::Signature("the model has no input".to_owned()))?;
    match input.dtype() {
        ValueType::Tensor {
            shape,
            dimension_symbols,
            ..
        } if shape.len() == 4 => Ok(shape
            .iter()
            .zip(dimension_symbols.iter())
            .map(|(&size, name)| (size < 0).then(|| name.clone()))
            .collect()),
        other => Err(OcrError::Signature(format!(
            "input `{}` is not a 4-dimensional tensor: {other:?}",
            input.name()
        ))),
    }
}

/// The dimension overrides that pin every free dimension of an input to
/// `shape`.
///
/// # Errors
///
/// [`OcrError::Signature`] if a free dimension has no name (it cannot be
/// overridden) or one name stands for two different sizes. A fixed
/// dimension that differs from `shape` is caught when the session's input
/// is compared with `shape` before its warm-up.
fn overrides(free: &[Option<String>], shape: [usize; 4]) -> Result<Vec<(String, usize)>, OcrError> {
    let mut pinned: Vec<(String, usize)> = Vec::new();
    for (position, (name, &size)) in free.iter().zip(&shape).enumerate() {
        let Some(name) = name else {
            continue;
        };
        if name.is_empty() {
            return Err(OcrError::Signature(format!(
                "input dimension {position} is dynamic but has no name to pin it by"
            )));
        }
        match pinned.iter().find(|(n, _)| n == name) {
            Some(&(_, other)) if other != size => {
                return Err(OcrError::Signature(format!(
                    "input dimension `{name}` would need the sizes {other} and {size}"
                )));
            }
            Some(_) => {}
            None => pinned.push((name.clone(), size)),
        }
    }
    Ok(pinned)
}

/// Builds one session with every free dimension pinned.
fn create_session(
    model: &[u8],
    dims: &[(String, usize)],
    target: Target,
    cpu_threads: usize,
) -> Result<FixedSession, OcrError> {
    const ACTION: &str = "creating an OCR session";
    let started = Instant::now();
    let providers: Vec<ExecutionProviderDispatch> = match target {
        Target::DirectMl(adapter) => {
            let dml = match adapter {
                Some(adapter) => ep::DirectML::default().with_device_id(adapter.dxgi_index as i32),
                None => ep::DirectML::default()
                    .with_device_filter(ep::directml::DeviceFilter::Gpu)
                    .with_performance_preference(
                        ep::directml::PerformancePreference::HighPerformance,
                    ),
            };
            vec![dml.build().error_on_failure()]
        }
        Target::Cpu => vec![ep::CPU::default().build().error_on_failure()],
    };
    let mut builder = Session::builder()
        .map_err(OcrError::session(ACTION))?
        .with_execution_providers(providers)
        .map_err(|e| OcrError::ProviderUnavailable {
            provider: match target {
                Target::Cpu => "CPU",
                Target::DirectMl(_) => "DirectML",
            },
            reason: e.to_string(),
        })?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(OcrError::session(ACTION))?
        // DirectML rules: no memory pattern, sequential execution.
        .with_memory_pattern(false)
        .map_err(OcrError::session(ACTION))?
        .with_parallel_execution(false)
        .map_err(OcrError::session(ACTION))?
        .with_intra_threads(match target {
            Target::Cpu => cpu_threads.max(1),
            Target::DirectMl(_) => 1,
        })
        .map_err(OcrError::session(ACTION))?;
    for (name, size) in dims {
        builder = builder
            .with_dimension_override(name, *size as i64)
            .map_err(OcrError::session(ACTION))?;
    }
    let session = builder
        .commit_from_memory(model)
        .map_err(OcrError::session(ACTION))?;
    let input = session
        .inputs()
        .first()
        .ok_or_else(|| OcrError::Signature("the model has no input".to_owned()))?;
    let shape: Vec<i64> = input
        .dtype()
        .tensor_shape()
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default();
    if shape.len() != 4 || shape.iter().any(|&d| d <= 0) {
        return Err(OcrError::Signature(format!(
            "input `{}` keeps dynamic dimensions after pinning: {shape:?}",
            input.name()
        )));
    }
    Ok(FixedSession {
        session,
        shape: [
            shape[0] as usize,
            shape[1] as usize,
            shape[2] as usize,
            shape[3] as usize,
        ],
        stats: ShapeStats {
            create_ms: started.elapsed().as_secs_f32() * 1000.0,
            // Nothing counts as slow before the warm-up sets the threshold.
            slow_threshold_ms: f32::INFINITY,
            ..ShapeStats::default()
        },
    })
}

/// Runs a new session once on zeros (compiling its graph on DirectML) and
/// records how long that took.
fn warm_up(
    mut fixed: FixedSession,
    shape: [usize; 4],
    target: Target,
    action: &'static str,
) -> Result<FixedSession, OcrError> {
    if fixed.shape != shape {
        return Err(OcrError::Signature(format!(
            "session input {:?} differs from the requested {shape:?}",
            fixed.shape
        )));
    }
    let input = vec![0.0f32; shape.iter().product()];
    let started = Instant::now();
    fixed.run(&input, action, |_, _| Ok(()))?;
    let warmup_ms = started.elapsed().as_secs_f32() * 1000.0;
    fixed.stats = ShapeStats {
        create_ms: fixed.stats.create_ms,
        warmup_ms,
        slow_threshold_ms: match target {
            Target::Cpu => f32::INFINITY,
            Target::DirectMl(_) => fixed.stats.create_ms + warmup_ms,
        },
        ..ShapeStats::default()
    };
    tracing::debug!(?shape, warmup_ms, "warmed up OCR session");
    Ok(fixed)
}

/// Number of classes in the recognition output (its last dimension).
fn output_classes(fixed: &FixedSession) -> Result<usize, OcrError> {
    let shape: Vec<i64> = fixed
        .session
        .outputs()
        .first()
        .and_then(|o| {
            o.dtype()
                .tensor_shape()
                .map(|s| s.iter().copied().collect())
        })
        .unwrap_or_default();
    match shape.last() {
        Some(&classes) if shape.len() == 3 && classes > 1 => Ok(classes as usize),
        _ => Err(OcrError::Signature(format!(
            "recognition output shape {shape:?} has no class dimension"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{ShapeStats, ZoneId, ZoneSpec, overrides};

    #[test]
    fn pins_free_dimensions_by_position() {
        let free = |names: [Option<&str>; 4]| names.map(|n| n.map(str::to_owned)).to_vec();
        // Detection: batch, height and width free.
        let det = free([Some("b"), None, Some("h"), Some("w")]);
        assert_eq!(
            overrides(&det, [1, 3, 96, 320]).unwrap(),
            [
                ("b".to_owned(), 1),
                ("h".to_owned(), 96),
                ("w".to_owned(), 320)
            ]
        );
        // The same name in two positions must mean one size.
        let shared = free([Some("n"), None, Some("n"), Some("w")]);
        assert!(overrides(&shared, [4, 3, 4, 160]).is_ok());
        assert!(overrides(&shared, [4, 3, 48, 160]).is_err());
        // A free dimension without a name cannot be pinned.
        assert!(overrides(&free([Some(""), None, None, None]), [1, 3, 48, 160]).is_err());
    }

    #[test]
    fn zone_sizes_round_up_to_multiples_of_32() {
        let spec = ZoneSpec::for_crop(ZoneId(1), 1260, 332, 2.0);
        assert_eq!((spec.content_width, spec.content_height), (2520, 664));
        assert_eq!((spec.width, spec.height), (2528, 672));
        let native = ZoneSpec::for_crop(ZoneId(1), 1260, 332, 1.0);
        assert_eq!((native.content_width, native.width), (1260, 1280));
        let small = ZoneSpec::for_crop(ZoneId(2), 10, 10, 1.0);
        assert_eq!((small.width, small.height), (32, 32));
    }

    #[test]
    fn tolerates_rare_stalls_only() {
        let mut stats = ShapeStats {
            slow_threshold_ms: 100.0,
            ..ShapeStats::default()
        };
        // One stall is tolerated at any run count.
        assert!(stats.record(500.0));
        assert!(stats.stalls_tolerable(), "{stats:?}");
        for _ in 0..1999 {
            assert!(!stats.record(5.0));
        }
        // Two in 2,001 runs are 0.1 %: tolerated.
        stats.record(500.0);
        assert_eq!((stats.runs, stats.slow_runs), (2001, 2));
        assert!(stats.stalls_tolerable(), "{stats:?}");
        // A third is above 0.1 %.
        stats.record(500.0);
        assert!(!stats.stalls_tolerable());
        // A shape compiled on every run fails at once.
        let mut recompiling = ShapeStats {
            slow_threshold_ms: 100.0,
            ..ShapeStats::default()
        };
        for _ in 0..3 {
            recompiling.record(500.0);
        }
        assert!(!recompiling.stalls_tolerable());
    }
}
