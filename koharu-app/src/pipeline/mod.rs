//! Pipeline: runs an ordered set of engines across one or more pages and
//! wraps each engine's output in one `Op::Batch` before applying via the
//! session's history.
//!
//! **Engines don't mutate the scene.** They return `Vec<Op>`; this driver
//! applies them transactionally (per-engine) against the active session.

pub mod artifacts;
pub mod engine;
mod engines;

pub use artifacts::Artifact;
pub use engine::{
    BoxFuture, Engine, EngineCtx, EngineInfo, EngineLoadFn, EngineResource, PipelineRunOptions,
    Registry, build_order,
};

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use koharu_core::{JobLogLevel, Op, PageId, PipelineStep};
use koharu_runtime::RuntimeManager;
use tokio::task::JoinSet;
use tracing::Instrument;

/// Observer for pipeline progress. `step_id` is the engine id of the step
/// about to run (or just finished); step_index / page_index are 0-based.
pub type ProgressSink = Arc<dyn Fn(ProgressTick) + Send + Sync>;

/// Observer for non-fatal step failures. Called once per failed step; the
/// pipeline skips the rest of that page's steps and moves on to the next
/// page.
pub type WarningSink = Arc<dyn Fn(WarningTick) + Send + Sync>;

/// Observer for per-step / per-page diagnostic logs (skip / done /
/// completion-decision). Distinct from `WarningSink` which is reserved for
/// step failures the user should notice.
pub type LogSink = Arc<dyn Fn(LogTick) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct ProgressTick {
    /// Coarse UI-facing step tag derived from the engine's primary
    /// produced artifact. `None` for the final 100% tick where no engine
    /// is running.
    pub step: Option<PipelineStep>,
    /// Engine id (e.g. `"paddle-ocr-vl-1.5"`) for diagnostics + logs.
    pub step_id: String,
    pub step_index: usize,
    pub total_steps: usize,
    pub page_index: usize,
    pub total_pages: usize,
    pub overall_percent: u8,
}

#[derive(Debug, Clone)]
pub struct WarningTick {
    pub step_id: String,
    pub page_index: usize,
    pub total_pages: usize,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct LogTick {
    pub level: JobLogLevel,
    /// `None` for global / non-page-bound messages.
    pub page_index: Option<usize>,
    pub total_pages: usize,
    /// Engine id, when the message refers to a specific step.
    pub step_id: Option<String>,
    pub message: String,
    pub detail: Option<String>,
}

/// Returned by [`run`]. `warning_count == 0` means the run finished cleanly.
#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    pub warning_count: usize,
}

/// Map an engine's produced artifact to its UI step category. Stays
/// co-located with the engine metadata so adding a new engine can't
/// silently bypass the toolbar spinner — only the registered artifact
/// matters, not the engine's string id.
fn step_for(info: &EngineInfo) -> Option<PipelineStep> {
    info.produces.iter().find_map(|a| match a {
        Artifact::TextBoxes
        | Artifact::SegmentMask
        | Artifact::FontPredictions
        | Artifact::BubbleMask => Some(PipelineStep::Detect),
        Artifact::OcrText => Some(PipelineStep::Ocr),
        Artifact::Translations => Some(PipelineStep::LlmGenerate),
        Artifact::Inpainted => Some(PipelineStep::Inpaint),
        Artifact::FinalRender => Some(PipelineStep::Render),
        // Non-UI-facing artifacts (inputs, intermediate sprites) — no
        // toolbar step tag.
        _ => None,
    })
}

use crate::config::PipelineParallelismConfig;
use crate::llm;
use crate::renderer;
use crate::session::ProjectSession;

// ---------------------------------------------------------------------------
// Spec + scope
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PipelineSpec {
    pub scope: Scope,
    pub steps: Vec<String>,
    pub options: PipelineRunOptions,
    pub parallelism: PipelineParallelismConfig,
}

#[derive(Debug, Clone)]
pub enum Scope {
    WholeProject,
    Pages(Vec<PageId>),
}

#[derive(Debug, Clone)]
struct PageState {
    page_id: PageId,
    page_index: usize,
    next_step: usize,
    active: bool,
    running: bool,
    done: bool,
}

#[derive(Debug, Clone, Copy)]
struct ActiveLimits {
    max_pages_in_flight: usize,
    max_active_steps: usize,
    max_model_steps: usize,
    max_llm_steps: usize,
    max_render_steps: usize,
    max_same_engine_steps: usize,
}

impl ActiveLimits {
    fn from_config(config: &PipelineParallelismConfig) -> Self {
        Self {
            max_pages_in_flight: config.max_pages_in_flight.max(1),
            max_active_steps: config.max_active_steps.max(1),
            max_model_steps: config.max_model_steps.max(1),
            max_llm_steps: config.max_llm_steps.max(1),
            max_render_steps: config.max_render_steps.max(1),
            max_same_engine_steps: config.max_same_engine_steps.max(1),
        }
    }
}

#[derive(Default)]
struct RunningCounts {
    active_steps: usize,
    model_steps: usize,
    llm_steps: usize,
    render_steps: usize,
    engines: HashMap<&'static str, usize>,
}

impl RunningCounts {
    fn can_start(&self, info: &EngineInfo, limits: &ActiveLimits) -> bool {
        if self.active_steps >= limits.max_active_steps {
            return false;
        }
        if self.engine_count(info.id) >= limits.max_same_engine_steps {
            return false;
        }
        match info.resource {
            EngineResource::Model => self.model_steps < limits.max_model_steps,
            EngineResource::Llm => self.llm_steps < limits.max_llm_steps,
            EngineResource::Render => self.render_steps < limits.max_render_steps,
        }
    }

    fn started(&mut self, info: &EngineInfo) {
        self.active_steps += 1;
        *self.engines.entry(info.id).or_insert(0) += 1;
        match info.resource {
            EngineResource::Model => self.model_steps += 1,
            EngineResource::Llm => self.llm_steps += 1,
            EngineResource::Render => self.render_steps += 1,
        }
    }

    fn finished(&mut self, info: &EngineInfo) {
        self.active_steps = self.active_steps.saturating_sub(1);
        if let Some(count) = self.engines.get_mut(info.id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.engines.remove(info.id);
            }
        }
        match info.resource {
            EngineResource::Model => self.model_steps = self.model_steps.saturating_sub(1),
            EngineResource::Llm => self.llm_steps = self.llm_steps.saturating_sub(1),
            EngineResource::Render => self.render_steps = self.render_steps.saturating_sub(1),
        }
    }

    fn engine_count(&self, engine_id: &str) -> usize {
        self.engines.get(engine_id).copied().unwrap_or(0)
    }
}

struct StepTask {
    page_slot: usize,
    page_id: PageId,
    page_index: usize,
    step_index: usize,
    engine_id: &'static str,
    session: Arc<ProjectSession>,
    registry: Arc<Registry>,
    runtime: Arc<RuntimeManager>,
    cpu: bool,
    llm: Arc<llm::Model>,
    renderer: Arc<renderer::Renderer>,
    options: PipelineRunOptions,
    cancel: Arc<AtomicBool>,
}

struct StepTaskResult {
    page_slot: usize,
    page_id: PageId,
    page_index: usize,
    step_index: usize,
    engine_id: &'static str,
    outcome: StepTaskOutcome,
}

enum StepTaskOutcome {
    LoadFailed(anyhow::Error),
    RunFailed {
        err: anyhow::Error,
        elapsed: Duration,
    },
    Success {
        ops: Vec<Op>,
        elapsed: Duration,
    },
}

impl StepTask {
    async fn run(self) -> StepTaskResult {
        let outcome = match self
            .registry
            .get(self.engine_id, &self.runtime, self.cpu)
            .await
        {
            Ok(engine) => {
                let scene_snap = self.session.scene_snapshot();
                let ctx = EngineCtx {
                    scene: &scene_snap,
                    page: self.page_id,
                    blobs: &self.session.blobs,
                    runtime: &self.runtime,
                    cancel: &self.cancel,
                    options: &self.options,
                    llm: &self.llm,
                    renderer: &self.renderer,
                };
                let started = Instant::now();
                let result = async { engine.run(ctx).await }
                    .instrument(tracing::info_span!(
                        "step",
                        engine = self.engine_id,
                        page = %self.page_id
                    ))
                    .await;
                let elapsed = started.elapsed();
                match result {
                    Ok(ops) => StepTaskOutcome::Success { ops, elapsed },
                    Err(err) => StepTaskOutcome::RunFailed { err, elapsed },
                }
            }
            Err(err) => StepTaskOutcome::LoadFailed(err),
        };

        StepTaskResult {
            page_slot: self.page_slot,
            page_id: self.page_id,
            page_index: self.page_index,
            step_index: self.step_index,
            engine_id: self.engine_id,
            outcome,
        }
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Execute `spec` against `session`. Each engine step becomes one `Op::Batch`
/// applied via the session's history (one undo step per step per page).
///
/// A failed step on a given page is non-fatal: the rest of that page's steps
/// are skipped (they typically depend on the failed step's output), one
/// [`WarningTick`] is emitted via `warnings`, and the driver moves on to the
/// next page. The function returns the total number of per-step warnings
/// that fired, letting callers flag the run as `CompletedWithErrors`.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(level = "info", skip_all)]
pub async fn run(
    session: Arc<ProjectSession>,
    registry: Arc<Registry>,
    runtime: Arc<RuntimeManager>,
    cpu: bool,
    llm: Arc<llm::Model>,
    renderer: Arc<renderer::Renderer>,
    spec: PipelineSpec,
    cancel: Arc<AtomicBool>,
    progress: Option<ProgressSink>,
    warnings: Option<WarningSink>,
    logs: Option<LogSink>,
) -> Result<RunOutcome> {
    let infos: Vec<&EngineInfo> = spec
        .steps
        .iter()
        .map(|id| Registry::find(id))
        .collect::<Result<_>>()?;
    let order = build_order(&infos)?;

    let pages = match &spec.scope {
        Scope::WholeProject => session
            .scene
            .read()
            .pages
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        Scope::Pages(ids) => ids.clone(),
    };

    let total_pages = pages.len().max(1);
    let total_steps = order.len().max(1);
    let total_units = (total_pages * total_steps) as u64;
    let mut completed: u64 = 0;
    let mut warning_count: usize = 0;
    let limits = ActiveLimits::from_config(&spec.parallelism);

    let emit_log = |level: JobLogLevel,
                    page_index: Option<usize>,
                    step_id: Option<&str>,
                    message: String,
                    detail: Option<String>| {
        if let Some(sink) = logs.as_ref() {
            sink(LogTick {
                level,
                page_index,
                total_pages,
                step_id: step_id.map(|s| s.to_string()),
                message,
                detail,
            });
        }
    };

    emit_log(
        JobLogLevel::Info,
        None,
        None,
        format!(
            "Pipeline started: {} page(s), steps: {}, parallelism: pages={}, active={}, model={}, llm={}, render={}, same-engine={}",
            total_pages,
            spec.steps.join(" -> "),
            limits.max_pages_in_flight,
            limits.max_active_steps,
            limits.max_model_steps,
            limits.max_llm_steps,
            limits.max_render_steps,
            limits.max_same_engine_steps,
        ),
        None,
    );

    let mut page_states = pages
        .iter()
        .enumerate()
        .map(|(page_index, page_id)| PageState {
            page_id: *page_id,
            page_index,
            next_step: 0,
            active: false,
            running: false,
            done: false,
        })
        .collect::<Vec<_>>();
    let mut pending_pages = (0..page_states.len()).collect::<VecDeque<_>>();
    let mut ready_pages = VecDeque::new();
    let mut running = RunningCounts::default();
    let mut tasks = JoinSet::new();
    let mut active_pages = 0usize;

    loop {
        while active_pages < limits.max_pages_in_flight {
            if !activate_next_page(
                &mut pending_pages,
                &mut ready_pages,
                &mut page_states,
                &session,
                total_steps,
                &mut completed,
                &mut active_pages,
                &emit_log,
            ) {
                break;
            }
        }

        if active_pages == 0 && tasks.is_empty() && pending_pages.is_empty() {
            break;
        }

        let mut made_progress = false;
        if !cancel.load(Ordering::Relaxed) {
            let ready_count = ready_pages.len();
            for _ in 0..ready_count {
                let Some(page_slot) = ready_pages.pop_front() else {
                    break;
                };
                if page_states[page_slot].done || page_states[page_slot].running {
                    continue;
                }
                if page_states[page_slot].next_step >= order.len() {
                    mark_page_completed_if_ready(
                        &session,
                        page_states[page_slot].page_id,
                        page_states[page_slot].page_index,
                        total_pages,
                        &emit_log,
                    );
                    page_states[page_slot].done = true;
                    page_states[page_slot].active = false;
                    active_pages = active_pages.saturating_sub(1);
                    made_progress = true;
                    continue;
                }

                let seq = page_states[page_slot].next_step;
                let info = infos[order[seq]];
                if let Some(sink) = progress.as_ref() {
                    let percent = ((completed * 100) / total_units).min(100) as u8;
                    sink(ProgressTick {
                        step: step_for(info),
                        step_id: info.id.to_string(),
                        step_index: seq,
                        total_steps,
                        page_index: page_states[page_slot].page_index,
                        total_pages,
                        overall_percent: percent,
                    });
                }

                if !session
                    .scene
                    .read()
                    .pages
                    .contains_key(&page_states[page_slot].page_id)
                {
                    emit_log(
                        JobLogLevel::Warn,
                        Some(page_states[page_slot].page_index),
                        Some(info.id),
                        "skipped: page deleted mid-run".to_string(),
                        None,
                    );
                    completed += (total_steps - seq) as u64;
                    page_states[page_slot].done = true;
                    page_states[page_slot].active = false;
                    active_pages = active_pages.saturating_sub(1);
                    made_progress = true;
                    continue;
                }

                {
                    let scene_guard = session.scene.read();
                    if let Some(page) = scene_guard.pages.get(&page_states[page_slot].page_id)
                        && info.produces.iter().all(|a| a.ready(page))
                    {
                        let produced = info
                            .produces
                            .iter()
                            .map(|a| a.label())
                            .collect::<Vec<_>>()
                            .join(", ");
                        emit_log(
                            JobLogLevel::Info,
                            Some(page_states[page_slot].page_index),
                            Some(info.id),
                            format!("skipped: artifacts already satisfied ({produced})"),
                            None,
                        );
                        completed += 1;
                        page_states[page_slot].next_step += 1;
                        ready_pages.push_back(page_slot);
                        made_progress = true;
                        continue;
                    }
                }

                if !running.can_start(info, &limits) {
                    ready_pages.push_back(page_slot);
                    continue;
                }

                running.started(info);
                page_states[page_slot].running = true;
                tasks.spawn(
                    StepTask {
                        page_slot,
                        page_id: page_states[page_slot].page_id,
                        page_index: page_states[page_slot].page_index,
                        step_index: seq,
                        engine_id: info.id,
                        session: session.clone(),
                        registry: registry.clone(),
                        runtime: runtime.clone(),
                        cpu,
                        llm: llm.clone(),
                        renderer: renderer.clone(),
                        options: spec.options.clone(),
                        cancel: cancel.clone(),
                    }
                    .run(),
                );
                made_progress = true;
            }
        }

        if made_progress {
            continue;
        }

        if let Some(joined) = tasks.join_next().await {
            let result =
                joined.map_err(|err| anyhow::anyhow!("pipeline step task failed: {err}"))?;
            let info = Registry::find(result.engine_id)?;
            running.finished(info);
            page_states[result.page_slot].running = false;

            match result.outcome {
                StepTaskOutcome::LoadFailed(err) => {
                    emit_log(
                        JobLogLevel::Error,
                        Some(result.page_index),
                        Some(result.engine_id),
                        "engine load failed".to_string(),
                        Some(format!("{err:#}")),
                    );
                    report_step_failure(
                        result.engine_id,
                        &result.page_id,
                        result.step_index,
                        result.page_index,
                        total_pages,
                        total_steps,
                        &err,
                        &mut warning_count,
                        warnings.as_ref(),
                    );
                    completed += (total_steps - result.step_index) as u64;
                    page_states[result.page_slot].done = true;
                    page_states[result.page_slot].active = false;
                    active_pages = active_pages.saturating_sub(1);
                }
                StepTaskOutcome::RunFailed { err, elapsed } => {
                    emit_log(
                        JobLogLevel::Error,
                        Some(result.page_index),
                        Some(result.engine_id),
                        "step failed".to_string(),
                        Some(format!("{err:#}; elapsed {elapsed:.2?}")),
                    );
                    report_step_failure(
                        result.engine_id,
                        &result.page_id,
                        result.step_index,
                        result.page_index,
                        total_pages,
                        total_steps,
                        &err,
                        &mut warning_count,
                        warnings.as_ref(),
                    );
                    completed += (total_steps - result.step_index) as u64;
                    page_states[result.page_slot].done = true;
                    page_states[result.page_slot].active = false;
                    active_pages = active_pages.saturating_sub(1);
                }
                StepTaskOutcome::Success { ops, elapsed } => {
                    completed += 1;
                    if ops.is_empty() {
                        emit_log(
                            JobLogLevel::Info,
                            Some(result.page_index),
                            Some(result.engine_id),
                            format!("done in {:.2?} (no ops emitted)", elapsed),
                            None,
                        );
                        page_states[result.page_slot].next_step += 1;
                        ready_pages.push_back(result.page_slot);
                    } else {
                        let op_count = ops.len();
                        let batch = Op::Batch {
                            ops,
                            label: format!("{}: page {}", result.engine_id, result.page_id),
                        };
                        if let Err(err) = session.apply(batch) {
                            emit_log(
                                JobLogLevel::Error,
                                Some(result.page_index),
                                Some(result.engine_id),
                                "scene apply failed".to_string(),
                                Some(format!("{err:#}")),
                            );
                            report_step_failure(
                                result.engine_id,
                                &result.page_id,
                                result.step_index,
                                result.page_index,
                                total_pages,
                                total_steps,
                                &err,
                                &mut warning_count,
                                warnings.as_ref(),
                            );
                            completed += (total_steps - result.step_index - 1) as u64;
                            page_states[result.page_slot].done = true;
                            page_states[result.page_slot].active = false;
                            active_pages = active_pages.saturating_sub(1);
                        } else {
                            emit_log(
                                JobLogLevel::Info,
                                Some(result.page_index),
                                Some(result.engine_id),
                                format!(
                                    "done in {:.2?} ({} op{})",
                                    elapsed,
                                    op_count,
                                    if op_count == 1 { "" } else { "s" }
                                ),
                                None,
                            );
                            page_states[result.page_slot].next_step += 1;
                            ready_pages.push_back(result.page_slot);
                        }
                    }
                }
            }
            continue;
        }

        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        break;
    }

    if cancel.load(Ordering::Relaxed) {
        bail!("cancelled");
    }

    emit_log(
        JobLogLevel::Info,
        None,
        None,
        format!(
            "Pipeline finished: {} page(s), {} warning(s)",
            total_pages, warning_count
        ),
        None,
    );

    if let Some(sink) = progress.as_ref() {
        sink(ProgressTick {
            step: None,
            step_id: String::new(),
            step_index: total_steps.saturating_sub(1),
            total_steps,
            page_index: total_pages.saturating_sub(1),
            total_pages,
            overall_percent: 100,
        });
    }
    Ok(RunOutcome { warning_count })
}

#[allow(clippy::too_many_arguments)]
fn activate_next_page(
    pending_pages: &mut VecDeque<usize>,
    ready_pages: &mut VecDeque<usize>,
    page_states: &mut [PageState],
    session: &ProjectSession,
    total_steps: usize,
    completed: &mut u64,
    active_pages: &mut usize,
    emit_log: &impl Fn(JobLogLevel, Option<usize>, Option<&str>, String, Option<String>),
) -> bool {
    let Some(page_slot) = pending_pages.pop_front() else {
        return false;
    };
    page_states[page_slot].active = true;

    {
        let scene_guard = session.scene.read();
        if let Some(page) = scene_guard.pages.get(&page_states[page_slot].page_id)
            && page.completed
        {
            emit_log(
                JobLogLevel::Info,
                Some(page_states[page_slot].page_index),
                None,
                "skipped: page already marked completed".to_string(),
                None,
            );
            *completed += total_steps as u64;
            page_states[page_slot].done = true;
            page_states[page_slot].active = false;
            return true;
        }
    }

    *active_pages += 1;
    ready_pages.push_back(page_slot);
    true
}

fn mark_page_completed_if_ready(
    session: &ProjectSession,
    page_id: PageId,
    page_index: usize,
    total_pages: usize,
    emit_log: &impl Fn(JobLogLevel, Option<usize>, Option<&str>, String, Option<String>),
) {
    let scene_guard = session.scene.read();
    if let Some(page) = scene_guard.pages.get(&page_id) {
        if page.completed {
            emit_log(
                JobLogLevel::Info,
                Some(page_index),
                None,
                "page already marked completed".to_string(),
                None,
            );
        } else {
            let has_text = page
                .nodes
                .values()
                .any(|n| matches!(n.kind, koharu_core::NodeKind::Text(_)));
            if !has_text {
                drop(scene_guard);
                let _ = session.apply(Op::UpdatePage {
                    id: page_id,
                    patch: koharu_core::PagePatch {
                        completed: Some(true),
                        ..Default::default()
                    },
                    prev: koharu_core::PagePatch::default(),
                });
                emit_log(
                    JobLogLevel::Info,
                    Some(page_index),
                    None,
                    "marked completed (no text on page)".to_string(),
                    None,
                );
            } else {
                let final_ready = Artifact::FinalRender.ready(page);
                let sprites_ready = Artifact::RenderedSprites.ready(page);
                if final_ready && sprites_ready {
                    drop(scene_guard);
                    let _ = session.apply(Op::UpdatePage {
                        id: page_id,
                        patch: koharu_core::PagePatch {
                            completed: Some(true),
                            ..Default::default()
                        },
                        prev: koharu_core::PagePatch::default(),
                    });
                    emit_log(
                        JobLogLevel::Info,
                        Some(page_index),
                        None,
                        "marked completed".to_string(),
                        None,
                    );
                } else {
                    let mut missing = Vec::new();
                    for a in [
                        Artifact::TextBoxes,
                        Artifact::OcrText,
                        Artifact::FontPredictions,
                        Artifact::Translations,
                        Artifact::Inpainted,
                        Artifact::RenderedSprites,
                        Artifact::FinalRender,
                    ] {
                        if !a.ready(page) {
                            missing.push(a.label());
                        }
                    }
                    emit_log(
                        JobLogLevel::Warn,
                        Some(page_index),
                        None,
                        format!("not marked completed; missing: {}", missing.join(", ")),
                        None,
                    );
                }
            }
        }
    } else {
        emit_log(
            JobLogLevel::Warn,
            Some(page_index),
            None,
            "not marked completed; page deleted mid-run".to_string(),
            Some(format!("total pages: {total_pages}")),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn report_step_failure(
    engine_id: &str,
    page_id: &PageId,
    step_index: usize,
    page_index: usize,
    total_pages: usize,
    total_steps: usize,
    err: &anyhow::Error,
    warning_count: &mut usize,
    sink: Option<&WarningSink>,
) {
    let _ = total_steps;
    tracing::warn!(
        engine = engine_id,
        page = %page_id,
        step_index,
        "pipeline step failed: {err:#}"
    );
    *warning_count += 1;
    if let Some(sink) = sink {
        sink(WarningTick {
            step_id: engine_id.to_string(),
            page_index,
            total_pages,
            message: format!("{err:#}"),
        });
    }
}

// ---------------------------------------------------------------------------
// Engine catalog building (API surface)
// ---------------------------------------------------------------------------

use koharu_core::{EngineCatalog, EngineCatalogEntry};

/// Build the engine catalog DTO for the API.
pub fn catalog() -> EngineCatalog {
    let entry = |info: &&EngineInfo| EngineCatalogEntry {
        id: info.id.to_string(),
        name: info.name.to_string(),
        produces: info.produces.iter().map(|a| format!("{a:?}")).collect(),
    };
    EngineCatalog {
        detectors: Registry::providers(Artifact::TextBoxes)
            .iter()
            .map(entry)
            .collect(),
        font_detectors: Registry::providers(Artifact::FontPredictions)
            .iter()
            .map(entry)
            .collect(),
        segmenters: Registry::providers(Artifact::SegmentMask)
            .iter()
            .map(entry)
            .collect(),
        bubble_segmenters: Registry::providers(Artifact::BubbleMask)
            .iter()
            .map(entry)
            .collect(),
        ocr: Registry::providers(Artifact::OcrText)
            .iter()
            .map(entry)
            .collect(),
        translators: Registry::providers(Artifact::Translations)
            .iter()
            .map(entry)
            .collect(),
        inpainters: Registry::providers(Artifact::Inpainted)
            .iter()
            .map(entry)
            .collect(),
        renderers: Registry::providers(Artifact::FinalRender)
            .iter()
            .map(entry)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_includes_anime_text_detector() {
        let catalog = catalog();

        assert!(catalog.detectors.iter().any(|engine| {
            engine.id == "anime-text"
                && engine.name == "Anime Text YOLO (N)"
                && engine.produces.iter().map(String::as_str).eq(["TextBoxes"])
        }));
    }

    static MODEL_INFO: EngineInfo = EngineInfo {
        id: "test-model-limit",
        name: "Test Model Limit",
        needs: &[],
        produces: &[Artifact::TextBoxes],
        resource: EngineResource::Model,
        load: |_runtime, _cpu| Box::pin(async { unreachable!("metadata-only test") }),
    };

    static LLM_INFO: EngineInfo = EngineInfo {
        id: "test-llm-limit",
        name: "Test LLM Limit",
        needs: &[Artifact::OcrText],
        produces: &[Artifact::Translations],
        resource: EngineResource::Llm,
        load: |_runtime, _cpu| Box::pin(async { unreachable!("metadata-only test") }),
    };

    #[test]
    fn running_counts_enforce_resource_and_same_engine_limits() {
        let limits = ActiveLimits::from_config(&PipelineParallelismConfig {
            max_pages_in_flight: 2,
            max_active_steps: 2,
            max_model_steps: 1,
            max_llm_steps: 1,
            max_render_steps: 1,
            max_same_engine_steps: 1,
        });
        let mut running = RunningCounts::default();

        assert!(running.can_start(&MODEL_INFO, &limits));
        running.started(&MODEL_INFO);

        assert!(!running.can_start(&MODEL_INFO, &limits));
        assert!(running.can_start(&LLM_INFO, &limits));
        running.started(&LLM_INFO);
        assert!(!running.can_start(&LLM_INFO, &limits));

        running.finished(&MODEL_INFO);
        assert!(running.can_start(&MODEL_INFO, &limits));
    }
}
