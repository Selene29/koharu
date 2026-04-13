use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use koharu_app::AppResources;
use koharu_core::BootstrapStatus;
use koharu_runtime::RuntimeManager;
use tokio::sync::OnceCell;

#[derive(Clone)]
pub struct SharedState {
    inner: Arc<Inner>,
}

struct Inner {
    resources: Arc<OnceCell<AppResources>>,
    runtime: RuntimeManager,
    version: &'static str,
    bootstrap: Arc<tokio::sync::RwLock<BootstrapStatus>>,
    bootstrap_in_progress: AtomicBool,
}

impl SharedState {
    pub fn new(
        resources: Arc<OnceCell<AppResources>>,
        runtime: RuntimeManager,
        version: &'static str,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                resources,
                runtime,
                version,
                bootstrap: Arc::new(tokio::sync::RwLock::new(BootstrapStatus::loading())),
                bootstrap_in_progress: AtomicBool::new(false),
            }),
        }
    }

    pub fn get(&self) -> Option<AppResources> {
        self.inner.resources.get().cloned()
    }

    pub fn runtime(&self) -> RuntimeManager {
        self.inner.runtime.clone()
    }

    pub fn version(&self) -> &'static str {
        self.inner.version
    }

    pub async fn get_or_try_init<F, Fut>(&self, init: F) -> Result<AppResources>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<AppResources>>,
    {
        Ok(self.inner.resources.get_or_try_init(init).await?.clone())
    }

    pub async fn bootstrap_status(&self) -> BootstrapStatus {
        self.inner.bootstrap.read().await.clone()
    }

    pub async fn mark_bootstrap_loading(&self) {
        *self.inner.bootstrap.write().await = BootstrapStatus::loading();
    }

    pub async fn mark_bootstrap_ready(&self) {
        *self.inner.bootstrap.write().await = BootstrapStatus::ready();
    }

    pub async fn mark_bootstrap_retrying(&self, error: impl Into<String>) {
        *self.inner.bootstrap.write().await = BootstrapStatus::retrying(error);
    }

    pub fn try_begin_bootstrap(&self) -> bool {
        self.inner
            .bootstrap_in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn finish_bootstrap(&self) {
        self.inner
            .bootstrap_in_progress
            .store(false, Ordering::Release);
    }
}
