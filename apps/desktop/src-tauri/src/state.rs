//! The state the commands share, created once at start.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use blurrd_core::{AppError, ErrorKind, OcrCapability};
use blurrd_engine::{Engine, EngineConfig, OcrSetup, default_cache_dir};

use crate::capabilities::CapabilityProbe;
use crate::exact::ExactRenderer;
use crate::jobs::Jobs;
use crate::media_server::MediaServer;
use crate::paths::UserPaths;
use crate::presets::Presets;
use crate::projects::Projects;
use crate::scheme::{self, Pending, Workers};
use crate::session::Session;

/// File name of the user presets in the app config folder (plan 6.5).
const PRESETS_FILE: &str = "presets.json";
/// File name of the saved queue in the cache folder (plan 6.2).
const SESSION_FILE: &str = "session.json";
/// Folder of the cached capability probes inside the cache (plan 6.2).
const CAPS_FOLDER: &str = "caps";

/// Everything the commands need. Cheap to clone.
#[derive(Debug, Clone)]
pub(crate) struct AppState {
    /// The engine, or why it could not start (missing FFmpeg libraries).
    pub(crate) engine: Result<Arc<Engine>, AppError>,
    pub(crate) paths: Arc<UserPaths>,
    pub(crate) projects: Arc<Projects>,
    pub(crate) jobs: Arc<Jobs>,
    pub(crate) capabilities: Arc<CapabilityProbe>,
    pub(crate) presets: Arc<Presets>,
    pub(crate) session: Arc<Session>,
    pub(crate) exact: Arc<ExactRenderer>,
    /// The loopback media server, unless it could not start.
    pub(crate) media: Option<Arc<MediaServer>>,
    /// The threads answering `blurrd://` requests.
    pub(crate) scheme: Arc<Workers<Pending>>,
    /// The origin of the window's page, the only one allowed to read the
    /// scheme's and the media server's answers.
    pub(crate) origin: Arc<str>,
}

impl AppState {
    /// The state for an app whose config folder is `config_dir`, checking
    /// paths with `paths`, for a page served from `origin`. The engine caches in `%LOCALAPPDATA%\blurrd`
    /// and verifies with OCR when ONNX Runtime and the models are
    /// installed.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] when the job queue or the scheme's threads
    /// cannot start.
    pub(crate) fn new(config_dir: &Path, paths: UserPaths, origin: &str) -> Result<Self, AppError> {
        let mut config = EngineConfig::new(default_cache_dir());
        config.ocr = ocr_setup();
        let ocr = match &config.ocr {
            Some(_) => OcrCapability::NotProbed,
            None => OcrCapability::Unavailable {
                reason: "ONNX Runtime or the OCR models are not installed".to_owned(),
            },
        };
        let cache = config.cache_dir.clone();
        let media = match MediaServer::start(origin) {
            Ok(server) => Some(Arc::new(server)),
            Err(error) => {
                tauri_plugin_log::log::warn!(
                    "the media server could not start, videos play through the asset protocol: {error}"
                );
                None
            }
        };
        let workers = Workers::start(
            "blurrd-scheme",
            scheme::WORKERS,
            scheme::QUEUED,
            Pending::answer,
        )
        .map_err(|e| {
            AppError::new(ErrorKind::Internal, "blurrd could not start.").with_detail(e.to_string())
        })?;
        let engine = Engine::new(config).map(Arc::new);
        if let Err(error) = &engine {
            tauri_plugin_log::log::error!("the engine could not start: {error}");
        }
        Ok(Self {
            engine,
            paths: Arc::new(paths),
            projects: Arc::new(Projects::new(cache.clone())),
            jobs: Arc::new(Jobs::start()?),
            capabilities: Arc::new(CapabilityProbe::new(ocr)),
            presets: Arc::new(Presets::new(config_dir.join(PRESETS_FILE))),
            session: Arc::new(Session::new(cache.join(SESSION_FILE))),
            exact: Arc::new(ExactRenderer::new()),
            media,
            scheme: Arc::new(workers),
            origin: Arc::from(origin),
        })
    }

    /// The engine.
    ///
    /// # Errors
    ///
    /// Why it could not start.
    pub(crate) fn engine(&self) -> Result<Arc<Engine>, AppError> {
        self.engine.clone()
    }

    /// Starts the background capability probe.
    pub(crate) fn probe_capabilities(&self) {
        let caps = match &self.engine {
            Ok(engine) => engine.config().cache_dir.join(CAPS_FOLDER),
            Err(_) => default_cache_dir().join(CAPS_FOLDER),
        };
        self.capabilities.start(caps);
    }
}

/// ONNX Runtime and the OCR models: `ORT_DYLIB_PATH` and
/// `BLURRD_MODEL_DIR` when set (development), otherwise `onnxruntime.dll`
/// and `models` next to the executable (installed); `None` when either is
/// missing.
pub(crate) fn ocr_setup() -> Option<OcrSetup> {
    let next_to_exe = |name: &str| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
    };
    let from_env = |var: &str| std::env::var_os(var).map(PathBuf::from);
    let dylib = from_env("ORT_DYLIB_PATH").or_else(|| next_to_exe("onnxruntime.dll"))?;
    let models = from_env("BLURRD_MODEL_DIR").or_else(|| next_to_exe("models"))?;
    (dylib.is_file() && models.is_dir()).then(|| OcrSetup::new(dylib, models))
}
