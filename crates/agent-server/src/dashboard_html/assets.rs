use axum::http::StatusCode;
use std::collections::HashMap;
use std::path::Path;

use super::renderer::FALLBACK_DASHBOARD_HTML;
use super::types::{DashboardError, DashboardFault, DashboardFaultPolicy};

#[derive(Debug, Clone)]
pub struct DashboardAsset {
    pub name: String,
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct DashboardAssetRegistry {
    assets: HashMap<String, DashboardAsset>,
}

impl Default for DashboardAssetRegistry {
    fn default() -> Self {
        let mut registry = Self {
            assets: HashMap::new(),
        };

        // Seed with built-in embedded assets
        registry.register(
            "dashboard.html",
            "text/html; charset=utf-8",
            include_bytes!("../../static/dashboard.html").to_vec(),
        );
        registry.register(
            "fallback.html",
            "text/html; charset=utf-8",
            FALLBACK_DASHBOARD_HTML.as_bytes().to_vec(),
        );
        registry.register(
            "style.css",
            "text/css; charset=utf-8",
            b":root { --bg: #0f172a; --text: #f8fafc; } body { background: var(--bg); color: var(--text); }".to_vec(),
        );
        registry.register(
            "favicon.ico",
            "image/x-icon",
            vec![
                0, 0, 1, 0, 1, 0, 1, 1, 0, 0, 1, 0, 24, 0, 48, 0, 0, 0, 22, 0, 0, 0,
            ],
        );

        registry
    }
}

impl DashboardAssetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        content_type: &'static str,
        bytes: Vec<u8>,
    ) {
        let name_str = name.into();
        self.assets.insert(
            name_str.clone(),
            DashboardAsset {
                name: name_str,
                content_type,
                bytes,
            },
        );
    }

    pub fn get(&self, name: &str) -> Option<&DashboardAsset> {
        self.assets.get(name)
    }

    /// Loads an asset from an optional custom filesystem path, falling back to embedded registry.
    /// Injects faults if specified by the fault policy.
    pub fn load_asset(
        &self,
        name: &str,
        custom_dir: Option<&Path>,
        mut fault_policy: Option<&mut DashboardFaultPolicy>,
    ) -> Result<Vec<u8>, DashboardError> {
        if let Some(ref mut policy) = fault_policy {
            if let Some(fault) = policy.should_inject() {
                match fault {
                    DashboardFault::MissingAsset => {
                        return Err(DashboardError::MissingAsset(format!(
                            "Simulated fault: asset '{}' not found",
                            name
                        )));
                    }
                    DashboardFault::IoError => {
                        return Err(DashboardError::IoError(format!(
                            "Simulated fault: I/O read failure for asset '{}'",
                            name
                        )));
                    }
                    _ => {}
                }
            }
        }

        if let Some(dir) = custom_dir {
            let path = dir.join(name);
            match std::fs::read(&path) {
                Ok(bytes) => return Ok(bytes),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    return Err(DashboardError::MissingAsset(format!(
                        "Asset '{}' not found on filesystem at {:?}",
                        name, path
                    )));
                }
                Err(err) => {
                    return Err(DashboardError::IoError(format!(
                        "Filesystem I/O error reading asset '{}' at {:?}: {}",
                        name, path, err
                    )));
                }
            }
        }

        self.assets
            .get(name)
            .map(|a| a.bytes.clone())
            .ok_or_else(|| {
                DashboardError::MissingAsset(format!("Asset '{}' not found in registry", name))
            })
    }

    /// Resilient asset resolution: if custom disk I/O fails, safely falls back to embedded asset.
    /// If asset is completely missing, returns a 404 response with placeholder asset.
    pub fn load_asset_resilient(
        &self,
        name: &str,
        custom_dir: Option<&Path>,
        fault_policy: Option<&mut DashboardFaultPolicy>,
    ) -> (Vec<u8>, &'static str, StatusCode) {
        match self.load_asset(name, custom_dir, fault_policy) {
            Ok(bytes) => {
                let content_type = self
                    .assets
                    .get(name)
                    .map(|a| a.content_type)
                    .unwrap_or("application/octet-stream");
                (bytes, content_type, StatusCode::OK)
            }
            Err(DashboardError::IoError(_)) => {
                // Disk failure fallback: serve embedded fallback version
                if let Some(embedded) = self.assets.get(name) {
                    (
                        embedded.bytes.clone(),
                        embedded.content_type,
                        StatusCode::OK,
                    )
                } else {
                    (
                        b"<!-- Fallback: Asset unavailable due to I/O error -->".to_vec(),
                        "text/html; charset=utf-8",
                        StatusCode::INTERNAL_SERVER_ERROR,
                    )
                }
            }
            Err(DashboardError::MissingAsset(_)) => {
                // Missing asset: return structured 404 with placeholder payload
                (
                    format!("<!-- Missing asset: {} -->", name).into_bytes(),
                    "text/plain; charset=utf-8",
                    StatusCode::NOT_FOUND,
                )
            }
            Err(err) => (
                format!("<!-- Error loading asset: {} -->", err).into_bytes(),
                "text/plain; charset=utf-8",
                err.to_status_code(),
            ),
        }
    }
}
