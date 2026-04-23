//! Engine cache management.
//!
//! - `DELETE /engines/cache?engine={detect,ocr,inpaint,all}` — evict cached
//!   engine instances to free GPU/CPU memory.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::AppState;
use crate::error::{ApiError, ApiResult};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::default().routes(routes!(unload_engine))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct UnloadQuery {
    engine: String,
}

#[utoipa::path(
    delete,
    path = "/engines/cache",
    operation_id = "unloadEngine",
    tag = "system",
    params(UnloadQuery),
    responses(
        (status = 204),
        (status = 400, body = crate::error::ApiError),
    ),
)]
async fn unload_engine(
    State(app): State<AppState>,
    Query(q): Query<UnloadQuery>,
) -> ApiResult<StatusCode> {
    let engine = q.engine.to_lowercase();
    match engine.as_str() {
        "detect" => {
            let pipeline = app.config.load().pipeline.clone();
            app.registry.evict(&[
                pipeline.detector.as_str(),
                pipeline.segmenter.as_str(),
                pipeline.font_detector.as_str(),
                pipeline.bubble_segmenter.as_str(),
            ]);
            Ok(StatusCode::NO_CONTENT)
        }
        "ocr" => {
            let ocr = app.config.load().pipeline.ocr.clone();
            app.registry.evict(&[ocr.as_str()]);
            Ok(StatusCode::NO_CONTENT)
        }
        "inpaint" => {
            let inpainter = app.config.load().pipeline.inpainter.clone();
            app.registry.evict(&[inpainter.as_str()]);
            Ok(StatusCode::NO_CONTENT)
        }
        "all" => {
            app.registry.clear();
            Ok(StatusCode::NO_CONTENT)
        }
        other => Err(ApiError::bad_request(format!("unknown engine: {other}"))),
    }
}
