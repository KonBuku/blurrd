//! The `blurrd://` scheme (architecture plan 9.2), on Windows
//! `http://blurrd.localhost/<path>`:
//!
//! - `thumb/<project>/<index>.jpg`: a timeline thumbnail of the analysis;
//! - `exact/<project>/<frame>.json`: the decoded frame index, the project
//!   revision and the Exact patches of a frame ([`ExactFrame`]);
//! - `exact/<project>/<frame>/<roi>.jpg`: one patch (a `rev=` query only
//!   keeps the browser's cache apart per revision);
//!
//!   both Exact paths take the export's blur and enabled regions as JSON
//!   in a `settings=` query ([`ExactSettings`], percent-encoded), and use
//!   the project's own without it;
//! - `crop/<project>/<leak>.jpg?view=blurred|clear`: the crop of a leak
//!   of the verification report, blurred unless `clear` is asked for.
//!
//! Requests are answered by a few worker threads ([`Workers`]) off the
//! webview's thread; when [`QUEUED`] requests already wait, more answer
//! 503 at once. Unknown paths and projects answer 404, failures 500 with
//! the error message. Answers allow the window's origin only (CORS), as
//! the asset protocol's do.
//!
//! [`ExactFrame`]: blurrd_core::ExactFrame
//! [`ExactSettings`]: blurrd_core::ExactSettings

use std::path::Path;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

use blurrd_core::{AppError, ErrorKind, ExactSettings};
use tauri::http::{Request, Response, StatusCode, header};

use crate::projects::lock;
use crate::state::AppState;

/// The name of the scheme.
pub(crate) const SCHEME: &str = "blurrd";

/// Threads answering requests: thumbnails decode in parallel, Exact
/// requests take turns on the one frame seeker.
pub(crate) const WORKERS: usize = 4;
/// Requests that may wait for a worker.
pub(crate) const QUEUED: usize = 64;

/// A fixed set of threads that handle jobs of type `T` from a bounded
/// queue.
pub(crate) struct Workers<T> {
    queue: SyncSender<T>,
}

impl<T> std::fmt::Debug for Workers<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workers").finish_non_exhaustive()
    }
}

impl<T: Send + 'static> Workers<T> {
    /// `threads` threads named `name` that call `handle` for every job,
    /// with room for `queued` waiting jobs.
    ///
    /// # Errors
    ///
    /// The error of starting a thread.
    pub(crate) fn start(
        name: &str,
        threads: usize,
        queued: usize,
        handle: impl Fn(T) + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let (queue, jobs) = sync_channel(queued);
        let jobs: Arc<Mutex<Receiver<T>>> = Arc::new(Mutex::new(jobs));
        let handle = Arc::new(handle);
        for _ in 0..threads {
            let jobs = Arc::clone(&jobs);
            let handle = Arc::clone(&handle);
            std::thread::Builder::new()
                .name(name.to_owned())
                .spawn(move || {
                    loop {
                        // The lock is held only while waiting for a job.
                        let job = lock(&jobs).recv();
                        match job {
                            Ok(job) => handle(job),
                            Err(_) => return,
                        }
                    }
                })?;
        }
        Ok(Self { queue })
    }

    /// Queues `job`; gives it back when the queue is full.
    pub(crate) fn submit(&self, job: T) -> Result<(), T> {
        self.queue.try_send(job).map_err(|error| match error {
            TrySendError::Full(job) | TrySendError::Disconnected(job) => job,
        })
    }
}

/// A request waiting for a worker.
pub(crate) struct Pending {
    state: AppState,
    request: Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
}

impl Pending {
    /// A request of `state`'s app to answer through `responder`.
    pub(crate) fn new(
        state: AppState,
        request: Request<Vec<u8>>,
        responder: tauri::UriSchemeResponder,
    ) -> Self {
        Self {
            state,
            request,
            responder,
        }
    }

    /// Answers the request.
    pub(crate) fn answer(self) {
        self.responder.respond(respond(&self.state, &self.request));
    }

    /// Answers 503: too many requests wait.
    pub(crate) fn busy(self) {
        let busy = status(StatusCode::SERVICE_UNAVAILABLE, "busy");
        self.responder.respond(for_window(busy, &self.state.origin));
    }
}

/// What a request asks for.
#[derive(Debug, Clone, PartialEq)]
enum Route {
    Thumbnail {
        project: String,
        index: u32,
    },
    ExactFrame {
        project: String,
        frame: u32,
        settings: Option<ExactSettings>,
    },
    ExactPatch {
        project: String,
        frame: u32,
        roi: String,
        settings: Option<ExactSettings>,
    },
    LeakCrop {
        project: String,
        index: usize,
        clear: bool,
    },
}

impl Route {
    /// The route of a request URI. `convertFileSrc` encodes the whole
    /// path, its `/` included, so the path is decoded first.
    fn of(uri: &tauri::http::Uri) -> Option<Self> {
        let path = String::from_utf8(percent_decoded(uri.path())?).ok()?;
        Self::parse(path.trim_start_matches('/'), uri.query())
    }

    /// The route of a request path (without the leading `/`) and query;
    /// `None` for unknown paths and invalid settings.
    fn parse(path: &str, query: Option<&str>) -> Option<Self> {
        let parts: Vec<&str> = path.split('/').collect();
        let pairs = || query.unwrap_or_default().split('&');
        let settings = || -> Option<Option<ExactSettings>> {
            match pairs().find_map(|pair| pair.strip_prefix("settings=")) {
                None => Some(None),
                Some(value) => serde_json::from_slice(&percent_decoded(value)?)
                    .ok()
                    .map(Some),
            }
        };
        match parts.as_slice() {
            ["thumb", project, file] => Some(Self::Thumbnail {
                project: (*project).to_owned(),
                index: file.strip_suffix(".jpg")?.parse().ok()?,
            }),
            ["exact", project, file] => Some(Self::ExactFrame {
                project: (*project).to_owned(),
                frame: file.strip_suffix(".json")?.parse().ok()?,
                settings: settings()?,
            }),
            ["exact", project, frame, file] => Some(Self::ExactPatch {
                project: (*project).to_owned(),
                frame: frame.parse().ok()?,
                roi: file.strip_suffix(".jpg")?.to_owned(),
                settings: settings()?,
            }),
            ["crop", project, file] => Some(Self::LeakCrop {
                project: (*project).to_owned(),
                index: file.strip_suffix(".jpg")?.parse().ok()?,
                clear: pairs().any(|pair| pair == "view=clear"),
            }),
            _ => None,
        }
    }
}

/// Answers one request of the scheme, for the window's origin.
pub(crate) fn respond(state: &AppState, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    for_window(answer(state, request), &state.origin)
}

/// `response` readable by the page of the window at `origin` (CORS).
fn for_window(mut response: Response<Vec<u8>>, origin: &str) -> Response<Vec<u8>> {
    if let Ok(origin) = header::HeaderValue::from_str(origin) {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    response
}

/// The answer to one request, without CORS headers.
fn answer(state: &AppState, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let Some(route) = Route::of(request.uri()) else {
        return status(StatusCode::NOT_FOUND, "unknown path");
    };
    let answer = match route {
        Route::Thumbnail { project, index } => state
            .projects
            .get(&project)
            .and_then(|project| read(&project.dir().thumbnail(index)))
            .map(|jpeg| ("image/jpeg", jpeg)),
        Route::ExactFrame {
            project,
            frame,
            settings,
        } => state
            .projects
            .get(&project)
            .and_then(|project| state.exact.frame(&project, frame, settings.as_ref()))
            .and_then(|frame| {
                serde_json::to_vec(&frame).map_err(|e| {
                    AppError::new(ErrorKind::Internal, "The patches could not be listed.")
                        .with_detail(e.to_string())
                })
            })
            .map(|json| ("application/json", json)),
        Route::ExactPatch {
            project,
            frame,
            roi,
            settings,
        } => state
            .projects
            .get(&project)
            .and_then(|project| state.exact.patch(&project, frame, &roi, settings.as_ref()))
            .map(|jpeg| ("image/jpeg", jpeg)),
        Route::LeakCrop {
            project,
            index,
            clear,
        } => state
            .projects
            .get(&project)
            .and_then(|project| state.exact.leak_crop(&project, index, clear))
            .map(|jpeg| ("image/jpeg", jpeg)),
    };
    match answer {
        Ok((content_type, body)) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CACHE_CONTROL, "no-cache")
            .body(body)
            .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR, "invalid response")),
        Err(error) => {
            let code = match error.kind {
                ErrorKind::NotFound | ErrorKind::InvalidProject => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            status(code, &error.to_string())
        }
    }
}

/// The bytes of a percent-encoded query value (`+` stays as it is, as
/// `encodeURIComponent` never writes it for a space); `None` for a broken
/// escape.
fn percent_decoded(value: &str) -> Option<Vec<u8>> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

fn read(path: &Path) -> Result<Vec<u8>, AppError> {
    std::fs::read(path).map_err(|e| {
        let kind = if e.kind() == std::io::ErrorKind::NotFound {
            ErrorKind::NotFound
        } else {
            ErrorKind::Io
        };
        AppError::new(kind, "The thumbnail could not be read.")
            .with_detail(format!("{}: {e}", path.display()))
    })
}

/// The answer while the app is still starting.
pub(crate) fn unavailable() -> Response<Vec<u8>> {
    status(StatusCode::SERVICE_UNAVAILABLE, "blurrd is starting")
}

/// A plain-text response with `code`.
fn status(code: StatusCode, message: &str) -> Response<Vec<u8>> {
    let mut response = Response::new(message.as_bytes().to_vec());
    *response.status_mut() = code;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use blurrd_core::{BlurParams, ExactSettings, Region, RegionSet, Strength};

    use super::{Route, Workers, for_window, percent_decoded, status};
    use crate::media::encode_component;

    #[test]
    fn answers_allow_the_window_origin_only() {
        let answer = for_window(
            status(tauri::http::StatusCode::NOT_FOUND, "x"),
            "http://tauri.localhost",
        );
        assert_eq!(
            answer.headers()["access-control-allow-origin"],
            "http://tauri.localhost"
        );
    }

    /// Two workers and two queue slots: a fifth job waits nowhere.
    #[test]
    fn workers_are_bounded_and_answer_every_queued_job() {
        let gate = Arc::new(Mutex::new(()));
        let started = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(Mutex::new(Vec::new()));
        let closed = gate.lock().unwrap();
        let workers = {
            let (gate, started, done) =
                (Arc::clone(&gate), Arc::clone(&started), Arc::clone(&done));
            Workers::start("test-worker", 2, 2, move |job: u32| {
                started.fetch_add(1, Ordering::SeqCst);
                drop(gate.lock().unwrap());
                done.lock().unwrap().push(job);
            })
            .unwrap()
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        // Two jobs block both workers ...
        assert!(workers.submit(0).is_ok());
        assert!(workers.submit(1).is_ok());
        while started.load(Ordering::SeqCst) < 2 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        // ... two wait in the queue, the rest is turned away.
        let accepted: Vec<u32> = (2..8).filter(|&job| workers.submit(job).is_ok()).collect();
        assert_eq!(accepted, vec![2, 3]);
        drop(closed);
        while done.lock().unwrap().len() < 4 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut finished = done.lock().unwrap().clone();
        finished.sort_unstable();
        assert_eq!(finished, vec![0, 1, 2, 3]);
    }

    #[test]
    fn paths_map_to_routes() {
        assert_eq!(
            Route::parse("thumb/ab12/00042.jpg", None),
            Some(Route::Thumbnail {
                project: "ab12".to_owned(),
                index: 42
            })
        );
        assert_eq!(
            Route::parse("exact/ab12/120.json", None),
            Some(Route::ExactFrame {
                project: "ab12".to_owned(),
                frame: 120,
                settings: None,
            })
        );
        assert_eq!(
            Route::parse("exact/ab12/120/p1.jpg", Some("rev=4")),
            Some(Route::ExactPatch {
                project: "ab12".to_owned(),
                frame: 120,
                roi: "p1".to_owned(),
                settings: None,
            })
        );
        assert_eq!(
            Route::parse("crop/ab12/3.jpg", Some("view=clear")),
            Some(Route::LeakCrop {
                project: "ab12".to_owned(),
                index: 3,
                clear: true
            })
        );
        assert_eq!(
            Route::parse("crop/ab12/3.jpg", Some("view=blurred")),
            Some(Route::LeakCrop {
                project: "ab12".to_owned(),
                index: 3,
                clear: false
            })
        );
        for bad in [
            "",
            "thumb/ab12",
            "thumb/ab12/x.jpg",
            "exact/ab12/1.png",
            "other/a/b.jpg",
        ] {
            assert_eq!(Route::parse(bad, None), None, "{bad}");
        }
    }

    #[test]
    fn encoded_uris_map_to_routes() {
        let route = |uri: &str| Route::of(&uri.parse().unwrap());
        assert_eq!(
            route("http://blurrd.localhost/thumb%2Fab12%2F7.jpg"),
            Some(Route::Thumbnail {
                project: "ab12".to_owned(),
                index: 7
            })
        );
        assert_eq!(
            route("http://blurrd.localhost/crop%2Fab12%2F3.jpg?view=clear"),
            Some(Route::LeakCrop {
                project: "ab12".to_owned(),
                index: 3,
                clear: true
            })
        );
        assert_eq!(
            route("http://blurrd.localhost/thumb/ab12/7.jpg"),
            route("http://blurrd.localhost/thumb%2Fab12%2F7.jpg")
        );
        assert_eq!(route("http://blurrd.localhost/thumb%2Fab12%2"), None);
    }

    /// The settings query as the frontend writes it.
    #[test]
    fn exact_routes_carry_the_export_settings() {
        let mut enabled = RegionSet::all();
        enabled.remove(Region::KillFeed);
        let settings = ExactSettings {
            blur: BlurParams {
                strength: Strength::Custom(9),
                ..BlurParams::default()
            },
            enabled,
        };
        let query = format!(
            "rev=3&settings={}",
            encode_component(&serde_json::to_string(&settings).unwrap())
        );
        assert_eq!(
            Route::parse("exact/ab12/7/p0.jpg", Some(&query)),
            Some(Route::ExactPatch {
                project: "ab12".to_owned(),
                frame: 7,
                roi: "p0".to_owned(),
                settings: Some(settings),
            })
        );
        assert_eq!(
            Route::parse("exact/ab12/7.json", Some(&query)),
            Some(Route::ExactFrame {
                project: "ab12".to_owned(),
                frame: 7,
                settings: Some(settings),
            })
        );
        for bad in ["settings=%7B", "settings=%zz", "settings=%4", "settings={}"] {
            assert_eq!(Route::parse("exact/ab12/7.json", Some(bad)), None, "{bad}");
        }
        assert_eq!(percent_decoded("a%20b+c"), Some(b"a b+c".to_vec()));
    }
}
