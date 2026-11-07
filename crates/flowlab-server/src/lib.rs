//! Local HTTP server: loads captures, runs analysis, answers the UI's queries and serves the
//! built UI. Binds to loopback by default; it has no authentication and can read any file
//! the user can (`POST /api/captures/open`), so it must not be exposed to a network.
//!
//! | Method | Path | Body / query | Response |
//! |---|---|---|---|
//! | GET | `/api/health` | | `{ version }` |
//! | GET | `/api/captures` | | `CaptureInfo[]` |
//! | POST | `/api/captures?name=` | raw file bytes | `CaptureInfo` |
//! | POST | `/api/captures/open` | `{ path }` | `CaptureInfo` |
//! | DELETE | `/api/captures/{id}` | | 204 |
//! | GET | `/api/captures/{id}/report` | | `CaptureReport` |
//! | GET | `/api/captures/{id}/frames` | `FrameFilter` + `offset`, `limit` | `FramePage` |
//! | GET | `/api/captures/{id}/frames/{no}` | | `FrameDetail` |
//! | GET | `/api/captures/{id}/eoe.pcapng` | | Ethernet frames reassembled from EoE (pcapng) |
//! | GET | `/api/captures/{id}/simulation` | | `SimResult` of a simulated capture |
//! | POST | `/api/captures/{id}/eni` | ENI XML | `CaptureInfo` (re-analysed with the configuration) |
//! | DELETE | `/api/captures/{id}/eni` | | `CaptureInfo` (re-analysed without) |
//! | POST | `/api/esi` | ESI XML | `EsiImport` |
//! | GET | `/api/plan/schema` | | JSON Schema of `PlanSpec` |
//! | GET | `/api/plan/example` | | `PlanSpec` |
//! | POST | `/api/plan` | `PlanSpec` | `PlanResult` |
//! | GET | `/api/synth/schema` | | JSON Schema of `SynthSpec` |
//! | GET | `/api/synth/example` | | `SynthSpec` |
//! | POST | `/api/synth` | `SynthSpec` | `CaptureInfo` |
//! | GET | `/api/sim/schema` | | JSON Schema of `SimSpec` |
//! | GET | `/api/sim/example` | | `SimSpec` |
//! | POST | `/api/simulate` | `SimSpec` | `CaptureInfo` (capture of the run; result via `/simulation`) |

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use flowlab_analysis::frames::{FrameFilter, detail, query};
use flowlab_analysis::{Analysis, AnalysisOptions, Severity, analyze};
use flowlab_capture::{CaptureFile, CaptureFormat, Trace};
use flowlab_core::Provenance;
use flowlab_devices::{EsiFile, NetworkConfig, PlanResult, PlanSlave, PlanSpec};
use flowlab_sim::{SimResult, SimSpec};
use flowlab_synth::SynthSpec;
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};
use ts_rs::TS;

/// Largest accepted upload.
pub const MAX_UPLOAD_BYTES: usize = 2 << 30;
/// Most frames a simulation run started from the UI may write into its capture.
pub const MAX_SIM_FRAMES: u64 = 500_000;

/// A loaded capture as listed by the API.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CaptureInfo {
    /// Server-assigned id.
    pub id: u32,
    /// File name.
    pub name: String,
    /// Provenance.
    pub provenance: Provenance,
    /// Container format.
    pub format: CaptureFormat,
    /// File size, bytes.
    pub bytes: u64,
    /// All frames.
    pub frames: u64,
    /// EtherCAT frames.
    pub ethercat_frames: u64,
    /// Duration, ns.
    pub duration_ns: f64,
    /// Cyclic streams found.
    pub streams: u32,
    /// Findings with severity `defect`.
    pub defects: u32,
    /// Findings with severity `warn`.
    pub warnings: u32,
    /// SHA-256 of the file.
    pub sha256: String,
    /// A simulation result is attached (`/api/captures/{id}/simulation`).
    pub has_simulation: bool,
    /// Name of the master in the attached network configuration (ENI), if any.
    pub eni: Option<String>,
}

/// Devices read from an ESI file, each with a plan entry for its default configuration.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct EsiImport {
    /// The file.
    pub file: EsiFile,
    /// One plan entry per device, in file order.
    pub slaves: Vec<PlanSlave>,
}

struct Entry {
    info: CaptureInfo,
    trace: Arc<Trace>,
    analysis: Analysis,
    simulation: Option<SimResult>,
}

/// Server state: loaded captures.
#[derive(Clone, Default)]
pub struct AppState {
    inner: Arc<RwLock<Store>>,
    options: Arc<AnalysisOptions>,
}

#[derive(Default)]
struct Store {
    next_id: u32,
    entries: Vec<Arc<Entry>>,
}

impl AppState {
    /// State with the given analysis options.
    pub fn new(options: AnalysisOptions) -> Self {
        Self { inner: Arc::default(), options: Arc::new(options) }
    }

    fn get(&self, id: u32) -> Result<Arc<Entry>, ApiError> {
        let store = self.inner.read().map_err(|_| ApiError::internal("state lock poisoned"))?;
        store
            .entries
            .iter()
            .find(|e| e.info.id == id)
            .cloned()
            .ok_or_else(|| ApiError::not_found(format!("no capture {id}")))
    }

    fn options_with(&self, network: Option<NetworkConfig>) -> AnalysisOptions {
        AnalysisOptions { network: network.or_else(|| self.options.network.clone()), ..(*self.options).clone() }
    }

    fn insert(&self, trace: Trace, analysis: Analysis, simulation: Option<SimResult>) -> Result<CaptureInfo, ApiError> {
        let mut store = self.inner.write().map_err(|_| ApiError::internal("state lock poisoned"))?;
        store.next_id += 1;
        let id = store.next_id;
        let entry = Self::entry(id, Arc::new(trace), analysis, simulation, self.options.network.as_ref());
        let info = entry.info.clone();
        store.entries.push(Arc::new(entry));
        Ok(info)
    }

    fn entry(
        id: u32,
        trace: Arc<Trace>,
        analysis: Analysis,
        simulation: Option<SimResult>,
        network: Option<&NetworkConfig>,
    ) -> Entry {
        let r = &analysis.report;
        let count = |s: Severity| r.findings.iter().filter(|f| f.severity == s).count() as u32;
        let info = CaptureInfo {
            id,
            name: trace.input.name.clone(),
            provenance: trace.provenance,
            format: trace.file.format,
            bytes: trace.input.bytes,
            frames: r.summary.frames,
            ethercat_frames: r.summary.ethercat_frames,
            duration_ns: r.summary.duration_ns as f64,
            streams: r.streams.len() as u32,
            defects: count(Severity::Defect),
            warnings: count(Severity::Warn),
            sha256: trace.input.sha256.clone(),
            has_simulation: simulation.is_some(),
            eni: network.map(|n| n.master.clone().unwrap_or_else(|| "network configuration".into())),
        };
        Entry { info, trace, analysis, simulation }
    }

    /// Re-analyse capture `id` with `network` (or without, when `None`) and replace it.
    pub fn set_network(&self, id: u32, network: Option<NetworkConfig>) -> Result<CaptureInfo, ApiError> {
        let old = self.get(id)?;
        let opts = self.options_with(network);
        let analysis = analyze(&old.trace, &opts);
        let entry = Self::entry(id, old.trace.clone(), analysis, old.simulation.clone(), opts.network.as_ref());
        let info = entry.info.clone();
        let mut store = self.inner.write().map_err(|_| ApiError::internal("state lock poisoned"))?;
        if let Some(slot) = store.entries.iter_mut().find(|e| e.info.id == id) {
            *slot = Arc::new(entry);
        }
        Ok(info)
    }

    /// Parse, index and analyze `bytes` and add the capture. Blocking; call off the runtime.
    pub fn load_bytes(&self, bytes: Vec<u8>, name: &str) -> Result<CaptureInfo, ApiError> {
        let file = CaptureFile::parse(bytes).map_err(|e| ApiError::bad_request(e.to_string()))?;
        let trace = Trace::build(file, name);
        let analysis = analyze(&trace, &self.options);
        self.insert(trace, analysis, None)
    }

    /// Run a simulation, analyse its capture and add both. Blocking.
    pub fn load_simulation(&self, mut spec: SimSpec) -> Result<CaptureInfo, ApiError> {
        spec.capture.enabled = true;
        spec.capture.max_frames = spec.capture.max_frames.min(MAX_SIM_FRAMES);
        let mut bytes = Vec::new();
        let result =
            flowlab_sim::simulate(&spec, Some(&mut bytes)).map_err(|e| ApiError::bad_request(e.to_string()))?;
        let file = CaptureFile::parse(bytes).map_err(|e| ApiError::internal(e.to_string()))?;
        let trace = Trace::build(file, &format!("sim-seed{}.pcapng", spec.seed));
        let analysis = analyze(&trace, &self.options);
        self.insert(trace, analysis, Some(result))
    }

    /// Read a capture from disk and add it. Blocking.
    pub fn load_path(&self, path: &Path) -> Result<CaptureInfo, ApiError> {
        let path = path.canonicalize().map_err(|e| ApiError::bad_request(format!("{}: {e}", path.display())))?;
        if !path.is_file() {
            return Err(ApiError::bad_request(format!("{} is not a file", path.display())));
        }
        let bytes = std::fs::read(&path).map_err(|e| ApiError::bad_request(format!("{}: {e}", path.display())))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.load_bytes(bytes, &name)
    }
}

/// An error with an HTTP status, rendered as `{ "error": "..." }`.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(m: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: m.into() }
    }
    fn not_found(m: impl Into<String>) -> Self {
        Self { status: StatusCode::NOT_FOUND, message: m.into() }
    }
    fn internal(m: impl Into<String>) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: m.into() }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ApiError> + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f).await.map_err(|e| ApiError::internal(e.to_string()))?
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }))
}

async fn list(State(s): State<AppState>) -> Result<Json<Vec<CaptureInfo>>, ApiError> {
    let store = s.inner.read().map_err(|_| ApiError::internal("state lock poisoned"))?;
    Ok(Json(store.entries.iter().map(|e| e.info.clone()).collect()))
}

#[derive(Deserialize)]
struct UploadQuery {
    name: Option<String>,
}

async fn upload(
    State(s): State<AppState>,
    Query(q): Query<UploadQuery>,
    body: axum::body::Bytes,
) -> Result<Json<CaptureInfo>, ApiError> {
    let name = q.name.unwrap_or_else(|| "upload.pcapng".to_owned());
    let name = Path::new(&name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Json(blocking(move || s.load_bytes(body.to_vec(), &name)).await?))
}

#[derive(Deserialize)]
struct OpenBody {
    path: PathBuf,
}

async fn open_path(State(s): State<AppState>, Json(b): Json<OpenBody>) -> Result<Json<CaptureInfo>, ApiError> {
    Ok(Json(blocking(move || s.load_path(&b.path)).await?))
}

async fn remove(State(s): State<AppState>, UrlPath(id): UrlPath<u32>) -> Result<StatusCode, ApiError> {
    let mut store = s.inner.write().map_err(|_| ApiError::internal("state lock poisoned"))?;
    let before = store.entries.len();
    store.entries.retain(|e| e.info.id != id);
    if store.entries.len() == before {
        return Err(ApiError::not_found(format!("no capture {id}")));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn report(State(s): State<AppState>, UrlPath(id): UrlPath<u32>) -> Result<Response, ApiError> {
    let e = s.get(id)?;
    Ok(Json(&e.analysis.report).into_response())
}

/// Query string of `/frames`. Spelled out instead of `#[serde(flatten)]`, which breaks
/// numeric and boolean fields in `serde_urlencoded`.
#[derive(Deserialize)]
struct FramesQuery {
    #[serde(default)]
    offset: u64,
    #[serde(default = "default_limit")]
    limit: u64,
    command: Option<String>,
    direction: Option<String>,
    station: Option<u16>,
    stream: Option<u16>,
    #[serde(default)]
    problems_only: bool,
    #[serde(default)]
    include_other: bool,
    mailbox: Option<String>,
}

impl FramesQuery {
    fn filter(&self) -> FrameFilter {
        FrameFilter {
            command: self.command.clone().filter(|c| !c.is_empty()),
            direction: self.direction.clone().filter(|d| !d.is_empty()),
            station: self.station,
            stream: self.stream,
            problems_only: self.problems_only,
            include_other: self.include_other,
            mailbox: self.mailbox.clone().filter(|m| !m.is_empty()),
        }
    }
}

fn default_limit() -> u64 {
    200
}

async fn frames(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<u32>,
    Query(q): Query<FramesQuery>,
) -> Result<Response, ApiError> {
    let e = s.get(id)?;
    let limit = q.limit.min(5000);
    let filter = q.filter();
    let page = blocking(move || Ok(query(&e.trace, &e.analysis, &filter, q.offset, limit))).await?;
    Ok(Json(page).into_response())
}

async fn frame(State(s): State<AppState>, UrlPath((id, no)): UrlPath<(u32, u32)>) -> Result<Response, ApiError> {
    let e = s.get(id)?;
    let d = detail(&e.trace, &e.analysis, no).ok_or_else(|| ApiError::not_found(format!("no frame {no}")))?;
    Ok(Json(d).into_response())
}

async fn eoe_pcapng(State(s): State<AppState>, UrlPath(id): UrlPath<u32>) -> Result<Response, ApiError> {
    let e = s.get(id)?;
    let stem = e.info.name.trim_end_matches(".pcapng").trim_end_matches(".pcap").to_owned();
    let bytes = blocking(move || {
        let mut out = Vec::new();
        flowlab_analysis::mailbox::write_eoe_pcapng(&e.analysis.mailbox, e.trace.provenance, &e.info.name, &mut out)
            .map_err(|err| ApiError::internal(err.to_string()))?;
        Ok(out)
    })
    .await?;
    let disposition = format!("attachment; filename=\"{}-eoe.pcapng\"", stem.replace(['"', '\\'], "_"));
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/vnd.tcpdump.pcap".to_owned()),
            (axum::http::header::CONTENT_DISPOSITION, disposition),
        ],
        bytes,
    )
        .into_response())
}

async fn simulation(State(s): State<AppState>, UrlPath(id): UrlPath<u32>) -> Result<Response, ApiError> {
    let e = s.get(id)?;
    match &e.simulation {
        Some(r) => Ok(Json(r).into_response()),
        None => Err(ApiError::not_found(format!("capture {id} is not a simulation run"))),
    }
}

fn xml_body(body: &axum::body::Bytes) -> Result<String, ApiError> {
    String::from_utf8(body.to_vec()).map_err(|_| ApiError::bad_request("the file is not UTF-8 XML"))
}

async fn attach_eni(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<u32>,
    body: axum::body::Bytes,
) -> Result<Json<CaptureInfo>, ApiError> {
    let text = xml_body(&body)?;
    let network = flowlab_devices::parse_eni(&text).map_err(|e| ApiError::bad_request(format!("ENI: {e}")))?;
    Ok(Json(blocking(move || s.set_network(id, Some(network))).await?))
}

async fn detach_eni(State(s): State<AppState>, UrlPath(id): UrlPath<u32>) -> Result<Json<CaptureInfo>, ApiError> {
    Ok(Json(blocking(move || s.set_network(id, None)).await?))
}

async fn esi(body: axum::body::Bytes) -> Result<Json<EsiImport>, ApiError> {
    let text = xml_body(&body)?;
    let file = flowlab_devices::parse_esi(&text).map_err(|e| ApiError::bad_request(format!("ESI: {e}")))?;
    let slaves = file.devices.iter().map(PlanSlave::from_device).collect();
    Ok(Json(EsiImport { file, slaves }))
}

async fn plan_schema() -> Json<serde_json::Value> {
    Json(flowlab_devices::plan_schema())
}

async fn plan_example() -> Json<PlanSpec> {
    Json(PlanSpec::example())
}

async fn plan(Json(spec): Json<PlanSpec>) -> Json<PlanResult> {
    Json(flowlab_devices::plan(&spec))
}

async fn sim_schema() -> Json<serde_json::Value> {
    Json(flowlab_sim::spec_schema())
}

async fn sim_example() -> Json<SimSpec> {
    Json(SimSpec::example())
}

async fn simulate(State(s): State<AppState>, Json(spec): Json<SimSpec>) -> Result<Json<CaptureInfo>, ApiError> {
    Ok(Json(blocking(move || s.load_simulation(spec)).await?))
}

async fn synth_schema() -> Json<serde_json::Value> {
    Json(flowlab_synth::spec_schema())
}

async fn synth_example() -> Json<SynthSpec> {
    Json(SynthSpec::example())
}

async fn synth(State(s): State<AppState>, Json(spec): Json<SynthSpec>) -> Result<Json<CaptureInfo>, ApiError> {
    Ok(Json(
        blocking(move || {
            let mut bytes = Vec::new();
            flowlab_synth::generate(&spec, &mut bytes).map_err(|e| ApiError::bad_request(e.to_string()))?;
            s.load_bytes(bytes, &format!("synth-seed{}.pcapng", spec.seed))
        })
        .await?,
    ))
}

/// The API router (no static files).
pub fn api(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/captures", get(list).post(upload))
        .route("/api/captures/open", post(open_path))
        .route("/api/captures/{id}", delete(remove))
        .route("/api/captures/{id}/report", get(report))
        .route("/api/captures/{id}/frames", get(frames))
        .route("/api/captures/{id}/frames/{no}", get(frame))
        .route("/api/captures/{id}/eoe.pcapng", get(eoe_pcapng))
        .route("/api/synth/schema", get(synth_schema))
        .route("/api/synth/example", get(synth_example))
        .route("/api/synth", post(synth))
        .route("/api/captures/{id}/simulation", get(simulation))
        .route("/api/sim/schema", get(sim_schema))
        .route("/api/sim/example", get(sim_example))
        .route("/api/simulate", post(simulate))
        .route("/api/captures/{id}/eni", post(attach_eni).delete(detach_eni))
        .route("/api/esi", post(esi))
        .route("/api/plan/schema", get(plan_schema))
        .route("/api/plan/example", get(plan_example))
        .route("/api/plan", post(plan))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
        .with_state(state)
}

/// API plus the built UI from `ui_dir`, with `index.html` as the SPA fallback.
pub fn app(state: AppState, ui_dir: Option<&Path>) -> Router {
    let router = api(state);
    match ui_dir {
        Some(dir) => {
            let index = dir.join("index.html");
            router.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => router,
    }
}

/// Bind and serve until Ctrl-C.
pub async fn serve(state: AppState, addr: SocketAddr, ui_dir: Option<PathBuf>) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app(state, ui_dir.as_deref()))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn call(app: &Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    fn json_post(uri: &str, v: &impl Serialize) -> Request<Body> {
        Request::post(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(v).unwrap()))
            .unwrap()
    }

    #[tokio::test]
    async fn synth_then_query() {
        let app = api(AppState::default());
        let mut spec = SynthSpec::example();
        spec.cycles = 300;
        let (st, info) = call(&app, json_post("/api/synth", &spec)).await;
        assert_eq!(st, StatusCode::OK, "{info}");
        assert_eq!(info["provenance"], "synthetic");
        let id = info["id"].as_u64().unwrap();

        let (st, list) = call(&app, Request::get("/api/captures").body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(list.as_array().unwrap().len(), 1);

        let (_, report) =
            call(&app, Request::get(format!("/api/captures/{id}/report")).body(Body::empty()).unwrap()).await;
        assert_eq!(report["meta"]["source"], "synthetic");
        assert_eq!(report["streams"].as_array().unwrap().len(), 1);

        let uri = format!(
            "/api/captures/{id}/frames?offset=10&limit=5&command=LRW&direction=tx&stream=0&problems_only=false&station="
        );
        let (st, _) = call(&app, Request::get(uri).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "empty numeric parameter is rejected");
        let uri = format!(
            "/api/captures/{id}/frames?offset=10&limit=5&command=LRW&direction=tx&stream=0&problems_only=false"
        );
        let (st, page) = call(&app, Request::get(uri).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::OK, "{page}");
        assert_eq!(page["rows"].as_array().unwrap().len(), 5);
        let no = page["rows"][0]["no"].as_u64().unwrap();

        let (st, d) =
            call(&app, Request::get(format!("/api/captures/{id}/frames/{no}")).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(d["datagrams"].as_array().unwrap().len(), 3);

        let (st, _) =
            call(&app, Request::get(format!("/api/captures/{id}/frames/0")).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::NOT_FOUND);

        let uri = format!("/api/captures/{id}/frames?mailbox=eoe&limit=1");
        let (st, page) = call(&app, Request::get(uri).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::OK);
        assert!(page["total"].as_u64().unwrap() > 0, "{page}");
        let res = app
            .clone()
            .oneshot(Request::get(format!("/api/captures/{id}/eoe.pcapng")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let eoe = CaptureFile::parse(bytes.to_vec()).unwrap();
        assert_eq!(eoe.packets.len() as u64, report["mailbox"]["eoe"]["frames_total"].as_u64().unwrap());
        let (st, _) = call(&app, Request::delete(format!("/api/captures/{id}")).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::NO_CONTENT);
        let (st, _) = call(&app, Request::get(format!("/api/captures/{id}/report")).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn simulate_then_read_result() {
        let app = api(AppState::default());
        let (st, example) = call(&app, Request::get("/api/sim/example").body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::OK);
        let mut spec: SimSpec = serde_json::from_value(example).unwrap();
        for t in &mut spec.transfers {
            t.payload_bytes = 4_000;
        }
        let (st, info) = call(&app, json_post("/api/simulate", &spec)).await;
        assert_eq!(st, StatusCode::OK, "{info}");
        assert_eq!(info["provenance"], "simulated");
        assert_eq!(info["has_simulation"], true);
        let id = info["id"].as_u64().unwrap();
        let (st, r) =
            call(&app, Request::get(format!("/api/captures/{id}/simulation")).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(r["meta"]["source"], "simulated");
        assert!(r["transfers"].as_array().unwrap().iter().all(|t| t["verified"] == true), "{r}");
        let (st, _) = call(&app, Request::get("/api/sim/schema").body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::OK);

        spec.slaves[2].dpram_bytes = 100;
        let (st, err) = call(&app, json_post("/api/simulate", &spec)).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(err["error"].as_str().unwrap().contains("ESC RAM"));
    }

    #[tokio::test]
    async fn eni_attach_esi_import_and_plan() {
        let app = api(AppState::default());
        let mut spec = SynthSpec::example();
        spec.cycles = 200;
        spec.faults.wkc_error_probability = 0.7;
        let (_, info) = call(&app, json_post("/api/synth", &spec)).await;
        let id = info["id"].as_u64().unwrap();
        assert!(info["eni"].is_null());
        let eni = flowlab_devices::write_eni(&flowlab_synth::network_config(&spec));
        let (st, info) =
            call(&app, Request::post(format!("/api/captures/{id}/eni")).body(Body::from(eni)).unwrap()).await;
        assert_eq!(st, StatusCode::OK, "{info}");
        assert_eq!(info["id"].as_u64(), Some(id), "same capture, re-analysed");
        assert!(info["eni"].as_str().unwrap().starts_with("flowlab-synth"));
        let (_, report) =
            call(&app, Request::get(format!("/api/captures/{id}/report")).body(Body::empty()).unwrap()).await;
        assert!(report["streams"][0]["wkc"].as_array().unwrap().iter().all(|w| w["expected_from"] == "eni"));
        let (st, _) =
            call(&app, Request::post(format!("/api/captures/{id}/eni")).body(Body::from("<x/>")).unwrap()).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let (st, info) =
            call(&app, Request::delete(format!("/api/captures/{id}/eni")).body(Body::empty()).unwrap()).await;
        assert_eq!(st, StatusCode::OK);
        assert!(info["eni"].is_null());

        let esi = include_str!("../../flowlab-devices/tests/fixtures/flowlab-demo.esi.xml");
        let (st, imp) = call(&app, Request::post("/api/esi").body(Body::from(esi)).unwrap()).await;
        assert_eq!(st, StatusCode::OK, "{imp}");
        assert_eq!(imp["slaves"][1]["mailbox_out_bytes"], 512);
        let (_, example) = call(&app, Request::get("/api/plan/example").body(Body::empty()).unwrap()).await;
        let (st, plan) = call(&app, json_post("/api/plan", &example)).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(plan["frames"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn upload_rejects_garbage_and_accepts_pcapng() {
        let app = api(AppState::default());
        let (st, err) =
            call(&app, Request::post("/api/captures?name=x.bin").body(Body::from(vec![1u8, 2, 3, 4, 5])).unwrap())
                .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(err["error"].as_str().unwrap().contains("pcap"));

        let mut spec = SynthSpec::example();
        spec.cycles = 50;
        let mut bytes = Vec::new();
        flowlab_synth::generate(&spec, &mut bytes).unwrap();
        let (st, info) =
            call(&app, Request::post("/api/captures?name=../../etc/demo.pcapng").body(Body::from(bytes)).unwrap())
                .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(info["name"], "demo.pcapng", "directories are stripped from names");
    }

    #[tokio::test]
    async fn open_path_errors_are_clean() {
        let app = api(AppState::default());
        let (st, err) =
            call(&app, json_post("/api/captures/open", &serde_json::json!({ "path": "/definitely/not/here.pcap" })))
                .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(err["error"].is_string());
    }
}
