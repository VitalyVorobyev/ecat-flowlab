//! Live capture endpoints: one session at a time, through `flowlab-live`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use flowlab_live::{CaptureInterface, ETHERCAT_FILTER, LiveOptions, LiveSession, LiveSnapshot, Source, Tool};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ApiError, AppState, CaptureInfo, blocking};

/// Live capture settings of the server.
#[derive(Debug, Clone)]
pub struct LiveConfig {
    /// Capturing from interfaces is allowed (`flowlab serve --live`). Replays always are.
    pub interfaces: bool,
    /// Where captures are saved.
    pub dir: PathBuf,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self { interfaces: false, dir: PathBuf::from("results/live") }
    }
}

#[derive(Default)]
pub(crate) enum Slot {
    #[default]
    Empty,
    Running(Box<LiveSession>),
    Done(Box<LiveSnapshot>),
}

pub(crate) type LiveSlot = Arc<Mutex<Slot>>;

/// What the server can capture from.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct LiveInterfaces {
    /// Capturing from interfaces is enabled (`flowlab serve --live`).
    pub enabled: bool,
    /// The capture tool that would be used.
    pub tool: Option<Tool>,
    /// Interfaces it can open.
    pub interfaces: Vec<CaptureInterface>,
    /// Why there are none.
    pub error: Option<String>,
}

/// Where a live session takes its packets from.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export)]
pub enum LiveSourceSpec {
    /// Capture on a network interface (needs `flowlab serve --live`). Only listens.
    Interface {
        /// Interface name.
        interface: String,
        /// Capture tool; default dumpcap, else tcpdump.
        #[serde(default)]
        tool: Option<Tool>,
        /// Keep all traffic, not only EtherCAT.
        #[serde(default)]
        all_traffic: bool,
    },
    /// Play a loaded capture back at its recorded pace.
    Replay {
        /// Id of a loaded capture.
        capture: u32,
        /// Playback speed (1 = real time).
        speed: f64,
    },
}

/// `POST /api/live/start` body.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct LiveStart {
    /// Where the packets come from.
    pub source: LiveSourceSpec,
    /// Capture time analysed in full each second, seconds (default 10).
    #[serde(default)]
    pub window_s: Option<f64>,
}

/// `POST /api/live/stop` response.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct LiveStopped {
    /// The final state.
    pub snapshot: LiveSnapshot,
    /// The saved capture, loaded for analysis (interface sessions that received packets).
    pub capture: Option<CaptureInfo>,
}

fn conflict(m: impl Into<String>) -> ApiError {
    ApiError { status: StatusCode::CONFLICT, message: m.into() }
}

fn lock(slot: &LiveSlot) -> std::sync::MutexGuard<'_, Slot> {
    slot.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) async fn interfaces(State(s): State<AppState>) -> Result<Json<LiveInterfaces>, ApiError> {
    let enabled = s.live_config.interfaces;
    blocking(move || {
        let found = flowlab_live::find_tool(None);
        let (tool, interfaces, error) = match found {
            Ok((t, path)) => match flowlab_live::list_interfaces(t, &path) {
                Ok(list) => (Some(t), list, None),
                Err(e) => (Some(t), Vec::new(), Some(e.to_string())),
            },
            Err(e) => (None, Vec::new(), Some(e.to_string())),
        };
        Ok(LiveInterfaces { enabled, tool, interfaces, error })
    })
    .await
    .map(Json)
}

pub(crate) async fn start(State(s): State<AppState>, Json(b): Json<LiveStart>) -> Result<Json<LiveSnapshot>, ApiError> {
    if matches!(*lock(&s.live), Slot::Running(_)) {
        return Err(conflict("a live session is running; stop it first"));
    }
    let window = match b.window_s {
        Some(w) if w > 0.0 && w <= 600.0 => Duration::from_secs_f64(w),
        Some(w) => return Err(ApiError::bad_request(format!("window_s must be in (0, 600], not {w}"))),
        None => LiveOptions::default().window,
    };
    let (source, save_to) = match b.source {
        LiveSourceSpec::Interface { interface, tool, all_traffic } => {
            if !s.live_config.interfaces {
                return Err(ApiError {
                    status: StatusCode::FORBIDDEN,
                    message: "capturing from interfaces is off; start the server with `flowlab serve --live`".into(),
                });
            }
            if interface.is_empty() || interface.starts_with('-') {
                return Err(ApiError::bad_request(format!("not an interface name: {interface:?}")));
            }
            let (t, path) = flowlab_live::find_tool(tool).map_err(|e| ApiError::bad_request(e.to_string()))?;
            let out = flowlab_live::default_save_path(&s.live_config.dir, t);
            let filter = (!all_traffic).then_some(ETHERCAT_FILTER);
            (Source::interface(t, path, &interface, filter), Some(out))
        }
        LiveSourceSpec::Replay { capture, speed } => {
            if speed.is_nan() || speed <= 0.0 || speed > 1000.0 {
                return Err(ApiError::bad_request(format!("speed must be in (0, 1000], not {speed}")));
            }
            let e = s.get(capture)?;
            (Source::Replay { bytes: e.trace.file.bytes.clone(), name: e.info.name.clone(), speed }, None)
        }
    };
    let options = LiveOptions {
        window,
        save_to,
        analysis: flowlab_analysis::AnalysisOptions { max_series_points: 300, ..(*s.options).clone() },
        ..LiveOptions::default()
    };
    let slot = s.live.clone();
    blocking(move || {
        let session = LiveSession::start(source, options).map_err(|e| ApiError::bad_request(e.to_string()))?;
        let snap = session.snapshot();
        let mut g = lock(&slot);
        if matches!(*g, Slot::Running(_)) {
            return Err(conflict("a live session is running; stop it first"));
        }
        *g = Slot::Running(Box::new(session));
        Ok(snap)
    })
    .await
    .map(Json)
}

pub(crate) async fn snapshot(State(s): State<AppState>) -> Result<Response, ApiError> {
    let slot = s.live.clone();
    let body = blocking(move || {
        let g = lock(&slot);
        let snap = match &*g {
            Slot::Empty => None,
            Slot::Running(session) => Some(session.snapshot()),
            Slot::Done(snap) => Some((**snap).clone()),
        };
        drop(g);
        serde_json::to_vec(&snap).map_err(|e| ApiError::internal(e.to_string()))
    })
    .await?;
    Ok(([(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response())
}

pub(crate) async fn stop(State(s): State<AppState>) -> Result<Json<LiveStopped>, ApiError> {
    blocking(move || {
        // Taken out first: the guard must be gone before the slot is locked again below.
        let taken = std::mem::take(&mut *lock(&s.live));
        let session = match taken {
            Slot::Running(session) => session,
            other => {
                *lock(&s.live) = other;
                return Err(conflict("no live session is running"));
            }
        };
        let snapshot = (*session).stop();
        *lock(&s.live) = Slot::Done(Box::new(snapshot.clone()));
        let capture = match &snapshot.saved_to {
            // Whatever arrived before a failure is worth keeping too.
            Some(p) if snapshot.packets > 0 => Some(s.load_path(std::path::Path::new(p))?),
            Some(p) => {
                let _ = std::fs::remove_file(p);
                None
            }
            None => None,
        };
        Ok(LiveStopped { snapshot, capture })
    })
    .await
    .map(Json)
}
