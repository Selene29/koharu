//! Project lifecycle routes. Every project lives under the managed
//! `{data.path}/projects/` directory; clients never supply filesystem
//! paths. A project's `id` is the `.khrproj/` directory basename.
//!
//! - `GET    /projects` — list managed projects
//! - `POST   /projects` — create a new project (`{name}`), server allocates path
//! - `POST   /projects/import` — extract a `.khr` archive into a fresh dir + open
//! - `PUT    /projects/current` — open a managed project by `id`
//! - `DELETE /projects/current` — close current session
//! - `POST   /projects/current/export` — export current; returns bytes

use std::sync::Arc;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use koharu_app::bus::EventBus;
use koharu_app::projects as project_dirs;
use koharu_core::{AppEvent, ImageRole, JobLogEvent, JobLogLevel, PageId, ProjectSummary};
use serde::{Deserialize, Serialize};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::default()
        .routes(routes!(list_projects))
        .routes(routes!(create_project))
        .routes(routes!(import_project))
        .routes(routes!(put_current_project))
        .routes(routes!(delete_current_project))
        .routes(routes!(export_current_project))
}

// ---------------------------------------------------------------------------
// GET /projects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListProjectsResponse {
    pub projects: Vec<ProjectSummary>,
}

#[utoipa::path(
    get,
    path = "/projects",
    responses((status = 200, body = ListProjectsResponse))
)]
async fn list_projects(State(app): State<AppState>) -> ApiResult<Json<ListProjectsResponse>> {
    let config = (**app.config.load()).clone();
    let projects = project_dirs::list_projects(&config).map_err(ApiError::internal)?;
    Ok(Json(ListProjectsResponse { projects }))
}

// ---------------------------------------------------------------------------
// POST /projects — create a new project from a display name
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectRequest {
    pub name: String,
}

#[utoipa::path(
    post,
    path = "/projects",
    request_body = CreateProjectRequest,
    responses((status = 200, body = ProjectSummary))
)]
async fn create_project(
    State(app): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> ApiResult<Json<ProjectSummary>> {
    let trimmed = req.name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request("name must not be empty"));
    }
    let config = (**app.config.load()).clone();
    let path = project_dirs::allocate_named(&config, trimmed).map_err(ApiError::internal)?;
    // `allocate_named` atomically created the directory so concurrent
    // callers can't collide. Session::create wants an empty-or-missing dir
    // and writes the scaffold — remove so it can populate.
    std::fs::remove_dir(path.as_std_path())
        .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?;
    let session = app
        .open_project(path, Some(trimmed.to_string()))
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(koharu_app::app::project_summary(&session)))
}

// ---------------------------------------------------------------------------
// PUT /projects/current — open a managed project by id
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpenProjectRequest {
    /// `.khrproj/` directory basename (no extension). Must exist under the
    /// managed projects directory.
    pub id: String,
}

#[utoipa::path(
    put,
    path = "/projects/current",
    request_body = OpenProjectRequest,
    responses((status = 200, body = ProjectSummary))
)]
async fn put_current_project(
    State(app): State<AppState>,
    Json(req): Json<OpenProjectRequest>,
) -> ApiResult<Json<ProjectSummary>> {
    let config = (**app.config.load()).clone();
    let path = project_dirs::project_path(&config, &req.id)
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    if !path.exists() {
        return Err(ApiError::not_found(format!("project {}", req.id)));
    }
    let session = app
        .open_project(path, None)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(koharu_app::app::project_summary(&session)))
}

#[utoipa::path(delete, path = "/projects/current", responses((status = 204)))]
async fn delete_current_project(State(app): State<AppState>) -> ApiResult<axum::http::StatusCode> {
    app.close_project().await.map_err(ApiError::internal)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// POST /projects/import — extract an archive into a fresh allocated dir
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/projects/import",
    request_body(content_type = "application/zip"),
    responses((status = 200, body = ProjectSummary))
)]
async fn import_project(
    State(app): State<AppState>,
    body: Bytes,
) -> ApiResult<Json<ProjectSummary>> {
    if body.is_empty() {
        return Err(ApiError::bad_request("empty archive body"));
    }
    let config = (**app.config.load()).clone();
    let dest =
        project_dirs::allocate_imported(&config, Some("imported")).map_err(ApiError::internal)?;
    // Atomic-created dir must be removed so `import_khr_bytes` can do its
    // own exists-check + populate.
    std::fs::remove_dir(dest.as_std_path())
        .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?;

    let body_vec = body.to_vec();
    let dest_c = dest.clone();
    tokio::task::spawn_blocking(move || koharu_app::archive::import_khr_bytes(&body_vec, &dest_c))
        .await
        .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?
        .map_err(ApiError::internal)?;

    let session = app
        .open_project(dest, None)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(koharu_app::app::project_summary(&session)))
}

// ---------------------------------------------------------------------------
// Export — returns bytes (zip when the format produces >1 file)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportProjectRequest {
    pub format: ExportFormat,
    /// Optional subset of pages; defaults to every page.
    #[serde(default)]
    pub pages: Option<Vec<PageId>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// Whole project as a `.khr` archive (always a single zip).
    Khr,
    /// One `.psd` per page.
    Psd,
    /// One `.png` per page (the Rendered layer).
    Rendered,
    /// One `.png` per page (the Inpainted layer).
    Inpainted,
}

#[utoipa::path(
    post,
    path = "/projects/current/export",
    request_body = ExportProjectRequest,
    responses((
        status = 200,
        content_type = "application/octet-stream",
        description = "Export bytes. Content-Type is `application/zip` when the format produces multiple files."
    ))
)]
async fn export_current_project(
    State(app): State<AppState>,
    Json(req): Json<ExportProjectRequest>,
) -> ApiResult<Response> {
    let session = app
        .current_session()
        .ok_or_else(|| ApiError::bad_request("no project open"))?;

    // Synthetic job id so the export's progress events flow through the
    // same SSE channel as pipeline jobs and land in the UI's activity log.
    let job_id = format!("export-{}", Uuid::new_v4());
    let format_label = match req.format {
        ExportFormat::Khr => "khr",
        ExportFormat::Psd => "psd",
        ExportFormat::Rendered => "rendered",
        ExportFormat::Inpainted => "inpainted",
    };
    let bus = app.bus.clone();
    emit_export_log(
        &bus,
        &job_id,
        format_label,
        JobLogLevel::Info,
        format!("export started: format={format_label}"),
        None,
    );

    let started = std::time::Instant::now();

    let s_for_compact = session.clone();
    emit_export_log(
        &bus,
        &job_id,
        format_label,
        JobLogLevel::Info,
        "compacting session before export".to_string(),
        None,
    );
    tokio::task::spawn_blocking(move || s_for_compact.compact())
        .await
        .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?
        .map_err(ApiError::internal)?;

    let project_name = session.scene.read().project.name.clone();

    let result = match req.format {
        ExportFormat::Khr => {
            emit_export_log(
                &bus,
                &job_id,
                format_label,
                JobLogLevel::Info,
                "packing project directory into .khr archive".to_string(),
                None,
            );
            let src = session.dir.clone();
            let bytes =
                tokio::task::spawn_blocking(move || koharu_app::archive::export_khr_bytes(&src))
                    .await
                    .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?
                    .map_err(ApiError::internal)?;
            Ok(bytes_response(
                bytes,
                &sanitize(&project_name, "project"),
                "khr",
                "application/octet-stream",
            ))
        }
        ExportFormat::Psd => {
            let page_ids = resolve_page_ids(&session, req.pages.as_deref())?;
            if page_ids.is_empty() {
                return Err(ApiError::bad_request("no pages in selection"));
            }
            let total = page_ids.len();
            emit_export_log(
                &bus,
                &job_id,
                format_label,
                JobLogLevel::Info,
                format!("rendering {total} PSD page(s)"),
                None,
            );
            let session_c = session.clone();
            let page_ids_c = page_ids.clone();
            let bus_c = bus.clone();
            let job_id_c = job_id.clone();
            let files = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                let mut out = Vec::with_capacity(page_ids_c.len());
                for (i, id) in page_ids_c.iter().enumerate() {
                    let t0 = std::time::Instant::now();
                    let bytes = crate::psd_export::psd_bytes_for_page(&session_c, *id)?;
                    out.push((format!("page-{:03}-{id}.psd", i + 1), bytes));
                    log_page_progress(
                        &bus_c,
                        &job_id_c,
                        "psd",
                        i + 1,
                        total,
                        t0.elapsed(),
                        out.last().map(|(_, b)| b.len()).unwrap_or(0),
                    );
                }
                Ok(out)
            })
            .await
            .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?
            .map_err(ApiError::internal)?;
            emit_export_log(
                &bus,
                &job_id,
                format_label,
                JobLogLevel::Info,
                format!("packing {} PSD file(s) into zip", files.len()),
                None,
            );
            Ok(files_to_response(files, &project_name, "psd")?)
        }
        ExportFormat::Rendered => {
            export_image_role(
                &session,
                req.pages.as_deref(),
                ImageRole::Rendered,
                &project_name,
                &bus,
                &job_id,
                format_label,
            )
            .await
        }
        ExportFormat::Inpainted => {
            export_image_role(
                &session,
                req.pages.as_deref(),
                ImageRole::Inpainted,
                &project_name,
                &bus,
                &job_id,
                format_label,
            )
            .await
        }
    };

    let elapsed = started.elapsed();
    match &result {
        Ok(_) => emit_export_log(
            &bus,
            &job_id,
            format_label,
            JobLogLevel::Info,
            format!("export complete in {elapsed:.2?}"),
            None,
        ),
        Err(err) => emit_export_log(
            &bus,
            &job_id,
            format_label,
            JobLogLevel::Error,
            format!("export failed after {elapsed:.2?}"),
            Some(format!("{}", err.message)),
        ),
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn export_image_role(
    session: &std::sync::Arc<koharu_app::ProjectSession>,
    pages: Option<&[PageId]>,
    role: ImageRole,
    project_name: &str,
    bus: &Arc<EventBus>,
    job_id: &str,
    format_label: &str,
) -> ApiResult<Response> {
    let page_ids = resolve_page_ids(session, pages)?;
    if page_ids.is_empty() {
        return Err(ApiError::bad_request("no pages in selection"));
    }
    let total = page_ids.len();
    emit_export_log(
        bus,
        job_id,
        format_label,
        JobLogLevel::Info,
        format!("rendering {total} {role:?} PNG page(s)"),
        None,
    );
    let session_c = session.clone();
    let page_ids_c = page_ids.clone();
    let bus_c = bus.clone();
    let job_id_c = job_id.to_string();
    let format_label_c = format_label.to_string();
    let files = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let mut out: Vec<(String, Vec<u8>)> = Vec::new();
        let mut fallbacks: usize = 0;
        for (i, id) in page_ids_c.iter().enumerate() {
            let t0 = std::time::Instant::now();
            // Pages without the requested role (e.g. textless cover art that
            // never produced a Rendered/Inpainted layer) fall back to the
            // Source image so they still appear in the export — otherwise
            // they would silently disappear from the user's output folder.
            let (bytes, used_source) =
                match crate::psd_export::png_bytes_for_page(&session_c, *id, role)? {
                    Some(b) => (b, false),
                    None => match crate::psd_export::png_bytes_for_page(
                        &session_c,
                        *id,
                        ImageRole::Source,
                    )? {
                        Some(b) => {
                            fallbacks += 1;
                            (b, true)
                        }
                        None => {
                            // No Source either — page genuinely has nothing
                            // to export. Skip silently.
                            continue;
                        }
                    },
                };
            let suffix = if used_source { "-source" } else { "" };
            let size = bytes.len();
            out.push((format!("page-{:03}-{id}{suffix}.png", i + 1), bytes));
            log_page_progress(
                &bus_c,
                &job_id_c,
                &format_label_c,
                i + 1,
                total,
                t0.elapsed(),
                size,
            );
        }
        if fallbacks > 0 {
            emit_export_log(
                &bus_c,
                &job_id_c,
                &format_label_c,
                JobLogLevel::Warn,
                format!(
                    "{fallbacks}/{total} page(s) used Source fallback (no {role:?} layer)"
                ),
                None,
            );
        }
        Ok(out)
    })
    .await
    .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?
    .map_err(ApiError::internal)?;

    if files.is_empty() {
        return Err(ApiError::bad_request(
            "no pages have the requested layer populated",
        ));
    }
    emit_export_log(
        bus,
        job_id,
        format_label,
        JobLogLevel::Info,
        format!("packing {} PNG file(s) into zip", files.len()),
        None,
    );
    files_to_response(files, project_name, role_ext(role))
}

fn emit_export_log(
    bus: &Arc<EventBus>,
    job_id: &str,
    format_label: &str,
    level: JobLogLevel,
    message: String,
    detail: Option<String>,
) {
    bus.publish(AppEvent::JobLog(JobLogEvent {
        job_id: job_id.to_string(),
        page_index: None,
        total_pages: 0,
        step_id: Some(format_label.to_string()),
        level,
        message,
        detail,
    }));
}

/// Throttled per-page progress: logs every page for small exports, every Nth
/// page for big ones, so the activity log stays informative without becoming
/// a wall of 200 lines.
fn log_page_progress(
    bus: &Arc<EventBus>,
    job_id: &str,
    format_label: &str,
    done: usize,
    total: usize,
    elapsed: std::time::Duration,
    bytes: usize,
) {
    let stride = if total <= 20 {
        1
    } else if total <= 100 {
        10
    } else {
        25
    };
    if done == total || done % stride == 0 {
        emit_export_log(
            bus,
            job_id,
            format_label,
            JobLogLevel::Info,
            format!(
                "page {done}/{total} ({:.1} KB in {elapsed:.2?})",
                bytes as f64 / 1024.0
            ),
            None,
        );
    }
}

fn resolve_page_ids(
    session: &koharu_app::ProjectSession,
    requested: Option<&[PageId]>,
) -> ApiResult<Vec<PageId>> {
    let scene = session.scene.read();
    match requested {
        None => Ok(scene.pages.keys().copied().collect()),
        Some(ids) => {
            for id in ids {
                if !scene.pages.contains_key(id) {
                    return Err(ApiError::not_found(format!("page {id}")));
                }
            }
            Ok(ids.to_vec())
        }
    }
}

fn role_ext(role: ImageRole) -> &'static str {
    match role {
        ImageRole::Rendered => "png",
        ImageRole::Inpainted => "png",
        ImageRole::Source => "png",
        ImageRole::Custom => "png",
    }
}

fn files_to_response(
    mut files: Vec<(String, Vec<u8>)>,
    project_name: &str,
    ext: &str,
) -> ApiResult<Response> {
    if files.len() == 1 {
        let (fname, bytes) = files.remove(0);
        let content_type = match ext {
            "psd" => "image/vnd.adobe.photoshop",
            "png" => "image/png",
            "khr" => "application/octet-stream",
            _ => "application/octet-stream",
        };
        return Ok(bytes_response_with_filename(bytes, &fname, content_type));
    }
    let zip_bytes = koharu_app::archive::zip_files_to_bytes(&files).map_err(ApiError::internal)?;
    let base = sanitize(project_name, "export");
    let filename = format!("{base}-{ext}.zip");
    Ok(bytes_response_with_filename(
        zip_bytes,
        &filename,
        "application/zip",
    ))
}

fn bytes_response(bytes: Vec<u8>, base: &str, ext: &str, content_type: &str) -> Response {
    let filename = format!("{base}.{ext}");
    bytes_response_with_filename(bytes, &filename, content_type)
}

fn bytes_response_with_filename(bytes: Vec<u8>, filename: &str, content_type: &str) -> Response {
    let cd = format!("attachment; filename=\"{filename}\"");
    let mut resp = Response::new(Body::from(bytes));
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    if let Ok(v) = HeaderValue::from_str(&cd) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    resp.into_response()
}

fn sanitize(name: &str, fallback: &str) -> String {
    let s: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}
