//! Engine trait + inventory-based registry + DAG resolver.
//!
//! An engine is a pluggable model that transforms one page. It declares the
//! artifacts it needs and produces; the DAG resolver derives execution order.
//!
//! **Engines emit ops, not mutations.** `run()` returns `Vec<Op>`; the driver
//! wraps them in `Op::Batch` and hands to `ProjectSession::apply`.
//!
//! ## Adding an engine
//!
//! 1. Define a struct holding your model.
//! 2. Implement `Engine` for it (returning `Vec<Op>`).
//! 3. Register via `inventory::submit! { EngineInfo { … } }` with a static
//!    async `load` function.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Result, bail};
use async_trait::async_trait;
use koharu_core::{NodeId, Op, PageId, Region, Scene};
use koharu_runtime::RuntimeManager;
use parking_lot::RwLock;
use petgraph::algo::toposort;
use petgraph::graph::DiGraph;
use tracing::Instrument;

use crate::blobs::BlobStore;
use crate::llm;
use crate::pipeline::artifacts::Artifact;
use crate::renderer;

// ---------------------------------------------------------------------------
// EngineCtx — everything an engine needs to produce ops
// ---------------------------------------------------------------------------

pub struct EngineCtx<'a> {
    /// A cheap clone of the target page (read-only).
    pub scene: &'a Scene,
    pub page: PageId,
    pub blobs: &'a BlobStore,
    pub runtime: &'a RuntimeManager,
    pub cancel: &'a AtomicBool,
    pub options: &'a PipelineRunOptions,
    pub llm: &'a llm::Model,
    pub renderer: &'a renderer::Renderer,
}

/// Options threaded through a pipeline run.
#[derive(Debug, Clone, Default)]
pub struct PipelineRunOptions {
    pub target_language: Option<String>,
    pub system_prompt: Option<String>,
    pub default_font: Option<String>,
    /// Optional text-node scope for engines that can operate on individual
    /// text blocks. Engines that render full-page artifacts ignore it.
    pub text_node_ids: Option<Vec<NodeId>>,
    /// Optional bounding-box hint. Inpainter engines (lama/aot) honor it:
    /// composite onto the existing `Image { Inpainted }` (fallback Source)
    /// and process just that one block. Other engines ignore it.
    pub region: Option<Region>,
}

// ---------------------------------------------------------------------------
// Engine trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait Engine: Send + Sync + 'static {
    /// Run the engine on one page. Return the ops to apply.
    /// Empty `Vec` = nothing changed (still a success).
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>>;
}

// ---------------------------------------------------------------------------
// EngineInfo — static descriptor + factory (registered via inventory)
// ---------------------------------------------------------------------------

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type EngineLoadFn =
    for<'a> fn(&'a RuntimeManager, bool) -> BoxFuture<'a, Result<Box<dyn Engine>>>;

pub struct EngineInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub needs: &'static [Artifact],
    pub produces: &'static [Artifact],
    pub resource: EngineResource,
    pub load: EngineLoadFn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineResource {
    Model,
    Llm,
    Render,
}

inventory::collect!(EngineInfo);

// ---------------------------------------------------------------------------
// Registry — lazy load + cache engine instances
// ---------------------------------------------------------------------------

pub struct Registry {
    engines: RwLock<HashMap<&'static str, Arc<dyn Engine>>>,
    load_locks: RwLock<HashMap<&'static str, Arc<tokio::sync::Mutex<()>>>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            engines: RwLock::new(HashMap::new()),
            load_locks: RwLock::new(HashMap::new()),
        }
    }
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or load an engine instance by id.
    pub async fn get(
        &self,
        id: &str,
        runtime: &RuntimeManager,
        cpu: bool,
    ) -> Result<Arc<dyn Engine>> {
        let info = Self::find(id)?;
        if let Some(engine) = self.engines.read().get(info.id).cloned() {
            return Ok(engine);
        }
        let load_lock = {
            if let Some(lock) = self.load_locks.read().get(info.id).cloned() {
                lock
            } else {
                let mut locks = self.load_locks.write();
                locks
                    .entry(info.id)
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                    .clone()
            }
        };
        let _guard = load_lock.lock().await;
        if let Some(engine) = self.engines.read().get(info.id).cloned() {
            return Ok(engine);
        }
        let loaded = async { (info.load)(runtime, cpu).await }
            .instrument(tracing::info_span!("engine_load", engine = id))
            .await?;
        let engine: Arc<dyn Engine> = Arc::from(loaded);
        self.engines.write().insert(info.id, engine.clone());
        Ok(engine)
    }

    /// Drop all cached engines (frees GPU memory).
    pub fn clear(&self) {
        self.engines.write().clear();
    }

    /// Remove specific engines from the cache by ID, freeing their resources.
    pub fn evict(&self, ids: &[&str]) {
        let mut engines = self.engines.write();
        for id in ids {
            engines.remove(*id);
        }
    }

    /// Find engine descriptor by id.
    pub fn find(id: &str) -> Result<&'static EngineInfo> {
        Self::catalog()
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow::anyhow!("unknown engine: {id}"))
    }

    /// All registered engine descriptors.
    pub fn catalog() -> Vec<&'static EngineInfo> {
        inventory::iter::<EngineInfo>.into_iter().collect()
    }

    /// Engines that produce a given artifact.
    pub fn providers(artifact: Artifact) -> Vec<&'static EngineInfo> {
        Self::catalog()
            .into_iter()
            .filter(|e| e.produces.contains(&artifact))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// DAG — derive execution order from artifact dependencies
// ---------------------------------------------------------------------------

/// Build a topological execution order from a set of engine infos.
pub fn build_order(infos: &[&EngineInfo]) -> Result<Vec<usize>> {
    let mut g = DiGraph::<usize, ()>::new();
    let mut id_to_node: HashMap<&str, _> = HashMap::new();

    for (i, info) in infos.iter().enumerate() {
        let n = g.add_node(i);
        if id_to_node.insert(info.id, n).is_some() {
            bail!("duplicate engine: {}", info.id);
        }
    }

    let mut producers: HashMap<Artifact, usize> = HashMap::new();
    for (i, info) in infos.iter().enumerate() {
        for &artifact in info.produces {
            producers.insert(artifact, i);
        }
    }

    for info in infos.iter() {
        let to = id_to_node[info.id];
        for &artifact in info.needs {
            if let Some(&producer) = producers.get(&artifact) {
                g.add_edge(id_to_node[infos[producer].id], to, ());
            }
        }
    }

    let order = toposort(&g, None)
        .map_err(|c| anyhow::anyhow!("cycle at '{}'", infos[g[c.node_id()]].id))?;
    Ok(order.into_iter().map(|n| g[n]).collect())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use anyhow::Result;
    use async_trait::async_trait;
    use koharu_core::Op;
    use koharu_runtime::{ComputePolicy, RuntimeManager};

    use super::*;

    static TEST_LOAD_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct TestEngine;

    #[async_trait]
    impl Engine for TestEngine {
        async fn run(&self, _ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
            Ok(Vec::new())
        }
    }

    inventory::submit! {
        EngineInfo {
            id: "test-load-lock-engine",
            name: "Test Load Lock Engine",
            needs: &[],
            produces: &[Artifact::TextBoxes],
            resource: EngineResource::Model,
            load: |_runtime, _cpu| Box::pin(async move {
                TEST_LOAD_COUNT.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                Ok(Box::new(TestEngine) as Box<dyn Engine>)
            }),
        }
    }

    #[tokio::test]
    async fn registry_loads_cold_engine_once_under_concurrency() -> Result<()> {
        TEST_LOAD_COUNT.store(0, Ordering::SeqCst);
        let temp = tempfile::tempdir()?;
        let runtime = RuntimeManager::new(temp.path(), ComputePolicy::CpuOnly)?;
        let registry = Arc::new(Registry::new());

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let registry = registry.clone();
            let runtime = runtime.clone();
            tasks.spawn(async move {
                registry
                    .get("test-load-lock-engine", &runtime, true)
                    .await
                    .map(|_| ())
            });
        }

        while let Some(result) = tasks.join_next().await {
            result??;
        }

        assert_eq!(TEST_LOAD_COUNT.load(Ordering::SeqCst), 1);
        Ok(())
    }
}
