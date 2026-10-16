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
//! Requests are answered on their own thread; unknown paths and projects
//! answer 404, failures 500 with the error message.
//!
//! [`ExactFrame`]: blurrd_core::ExactFrame
//! [`ExactSettings`]: blurrd_core::ExactSettings

use std::path::Path;

use blurrd_core::{AppError, ErrorKind, ExactSettings};
use tauri::http::{Request, Response, StatusCode, header};

use crate::state::AppState;

/// The name of the scheme.
pub(crate) const SCHEME: &str = "blurrd";

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

/// Answers one request of the scheme.
pub(crate) fn respond(state: &AppState, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let uri = request.uri();
    let path = uri.path().trim_start_matches('/');
    let Some(route) = Route::parse(path, uri.query()) else {
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
            // The page fetches from its own origin; only it can reach the scheme.
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
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
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        header::HeaderValue::from_static("*"),
    );
    response
}

#[cfg(test)]
mod tests {
    use blurrd_core::{BlurParams, ExactSettings, Region, RegionSet, Strength};

    use super::{Route, percent_decoded};
    use crate::media::encode_component;

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
