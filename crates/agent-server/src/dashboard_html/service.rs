use axum::http::StatusCode;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::assets::DashboardAssetRegistry;
use super::cache::{DashboardCacheStats, ShardedDashboardCache};
use super::renderer::{render_template, FALLBACK_DASHBOARD_HTML};
use super::types::{DashboardContext, DashboardError, DashboardFaultPolicy};

#[derive(Clone)]
pub struct ResilientDashboardService {
    max_retries: usize,
    backoff: Duration,
    custom_asset_dir: Option<PathBuf>,
    registry: DashboardAssetRegistry,
    fault_policy: Arc<Mutex<DashboardFaultPolicy>>,
    max_capacity: Option<usize>,
    history: VecDeque<String>,
    cache: Option<Arc<ShardedDashboardCache>>,
    cached_template: Arc<str>,
    rendered_count: usize,
    fallback_count: usize,
    error_count: usize,
}

impl Default for ResilientDashboardService {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff: Duration::from_millis(5),
            custom_asset_dir: None,
            registry: DashboardAssetRegistry::default(),
            fault_policy: Arc::new(Mutex::new(DashboardFaultPolicy::none())),
            max_capacity: None,
            history: VecDeque::new(),
            cache: None,
            cached_template: Arc::from(crate::dashboard_html::DASHBOARD_HTML),
            rendered_count: 0,
            fallback_count: 0,
            error_count: 0,
        }
    }
}

impl ResilientDashboardService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_retries(mut self, max: usize) -> Self {
        self.max_retries = max;
        self
    }

    pub fn with_backoff(mut self, backoff: Duration) -> Self {
        self.backoff = backoff;
        self
    }

    pub fn with_custom_asset_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.custom_asset_dir = Some(dir.into());
        self
    }

    pub fn with_fault_policy(mut self, policy: DashboardFaultPolicy) -> Self {
        self.fault_policy = Arc::new(Mutex::new(policy));
        self
    }

    pub fn with_max_capacity(mut self, max: usize) -> Self {
        self.max_capacity = Some(max);
        self
    }

    pub fn max_capacity(&self) -> Option<usize> {
        self.max_capacity
    }

    pub fn with_cache(mut self, capacity: usize) -> Self {
        self.cache = Some(Arc::new(ShardedDashboardCache::with_default_shards(
            capacity,
        )));
        self
    }

    pub fn with_shared_cache(mut self, cache: Arc<ShardedDashboardCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn cache(&self) -> Option<&Arc<ShardedDashboardCache>> {
        self.cache.as_ref()
    }

    pub fn cache_stats(&self) -> Option<DashboardCacheStats> {
        self.cache.as_ref().map(|c| c.stats())
    }

    pub fn fault_policy(&self) -> Arc<Mutex<DashboardFaultPolicy>> {
        self.fault_policy.clone()
    }

    pub fn rendered_count(&self) -> usize {
        self.rendered_count
    }

    pub fn fallback_count(&self) -> usize {
        self.fallback_count
    }

    pub fn error_count(&self) -> usize {
        self.error_count
    }

    pub fn reset_counts(&mut self) {
        self.rendered_count = 0;
        self.fallback_count = 0;
        self.error_count = 0;
    }

    pub fn record_history(&mut self, entry: String) {
        if let Some(cap) = self.max_capacity {
            if self.history.len() >= cap {
                self.history.pop_front();
            }
            self.history.push_back(entry);
        }
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base = std::mem::size_of::<Self>();
        let hist_bytes: usize = self
            .history
            .iter()
            .map(|s| s.len() + std::mem::size_of::<String>())
            .sum();
        let cache_bytes = self.cache.as_ref().map_or(0, |c| c.memory_usage_bytes());
        base + hist_bytes + cache_bytes
    }

    /// Renders dashboard with automatic retry for retryable transient faults (I/O, network).
    /// Optimized with zero-copy cached embedded template when no custom asset directory is configured.
    pub async fn render_dashboard(&self, ctx: &DashboardContext) -> Result<String, DashboardError> {
        let mut attempts = 0;

        loop {
            attempts += 1;

            // Check fault injection policy
            let fault = {
                let mut policy = self.fault_policy.lock().map_err(|_| {
                    DashboardError::IoError("Failed to acquire fault policy lock".to_string())
                })?;
                policy.should_inject()
            };

            if let Some(fault) = fault {
                let err = fault.to_dashboard_error();
                if !err.is_retryable() {
                    return Err(err);
                }
                if attempts > self.max_retries {
                    return Err(DashboardError::MaxRetriesExceeded(self.max_retries));
                }
                tokio::time::sleep(self.backoff).await;
                continue;
            }

            // Zero-copy path: if no custom asset dir, reuse cached embedded template directly
            if self.custom_asset_dir.is_none() {
                return render_template(&self.cached_template, ctx);
            }

            // Custom filesystem path with resilient fallback
            let raw_template = match self.registry.load_asset(
                "dashboard.html",
                self.custom_asset_dir.as_deref(),
                None,
            ) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(s) => s,
                    Err(e) => {
                        return Err(DashboardError::TemplateRenderFailure(format!(
                            "Invalid UTF-8 template encoding: {}",
                            e
                        )))
                    }
                },
                Err(err) => {
                    if !err.is_retryable() {
                        return Err(err);
                    }
                    if attempts > self.max_retries {
                        return Err(DashboardError::MaxRetriesExceeded(self.max_retries));
                    }
                    tokio::time::sleep(self.backoff).await;
                    continue;
                }
            };

            match render_template(&raw_template, ctx) {
                Ok(rendered) => return Ok(rendered),
                Err(err) => return Err(err),
            }
        }
    }

    /// Renders dashboard with cache lookup if configured.
    pub async fn render_dashboard_cached(
        &self,
        cache_key: &str,
        ctx: &DashboardContext,
    ) -> Result<String, DashboardError> {
        if let Some(cache) = &self.cache {
            if let Some(cached_html) = cache.get(cache_key) {
                return Ok(cached_html);
            }
        }

        let rendered = self.render_dashboard(ctx).await?;

        if let Some(cache) = &self.cache {
            cache.insert(cache_key.to_string(), rendered.clone());
        }

        Ok(rendered)
    }

    /// Resilient render: guarantees a valid HTML response even in catastrophic conditions.
    pub async fn render_dashboard_with_fallback(
        &self,
        ctx: &DashboardContext,
    ) -> (String, StatusCode) {
        match self.render_dashboard(ctx).await {
            Ok(html) => (html, StatusCode::OK),
            Err(err) => (FALLBACK_DASHBOARD_HTML.to_string(), err.to_status_code()),
        }
    }

    /// Batch rendering for high-throughput scenarios with zero-copy buffer optimization.
    pub fn render_batch(
        &self,
        contexts: &[DashboardContext],
    ) -> Vec<Result<String, DashboardError>> {
        if self.custom_asset_dir.is_none() {
            return contexts
                .iter()
                .map(|ctx| render_template(&self.cached_template, ctx))
                .collect();
        }

        let raw_template =
            match self
                .registry
                .load_asset("dashboard.html", self.custom_asset_dir.as_deref(), None)
            {
                Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                Err(err) => return vec![Err(err); contexts.len()],
            };

        contexts
            .iter()
            .map(|ctx| render_template(&raw_template, ctx))
            .collect()
    }

    /// Serves an asset with resilient fallback to embedded storage on filesystem failure.
    pub async fn get_asset(&self, name: &str) -> (Vec<u8>, &'static str, StatusCode) {
        let mut attempts = 0;

        loop {
            attempts += 1;

            let (policy_fault, _) = {
                if let Ok(mut policy) = self.fault_policy.lock() {
                    (policy.should_inject(), ())
                } else {
                    (None, ())
                }
            };

            if let Some(fault) = policy_fault {
                let err = fault.to_dashboard_error();
                if err.is_retryable() && attempts <= self.max_retries {
                    tokio::time::sleep(self.backoff).await;
                    continue;
                }
                return (
                    format!("<!-- Error: {} -->", err).into_bytes(),
                    "text/plain; charset=utf-8",
                    err.to_status_code(),
                );
            }

            let result = self
                .registry
                .load_asset(name, self.custom_asset_dir.as_deref(), None);
            match result {
                Ok(bytes) => {
                    let content_type = self
                        .registry
                        .get(name)
                        .map(|a| a.content_type)
                        .unwrap_or("application/octet-stream");
                    return (bytes, content_type, StatusCode::OK);
                }
                Err(err) => {
                    if err.is_retryable() && attempts <= self.max_retries {
                        tokio::time::sleep(self.backoff).await;
                        continue;
                    }
                    return self.registry.load_asset_resilient(
                        name,
                        self.custom_asset_dir.as_deref(),
                        None,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
