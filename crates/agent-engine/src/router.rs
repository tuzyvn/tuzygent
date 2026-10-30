use crate::cli::{AntigravityEngine, ClaudeCodeEngine, GenericCliEngine};
use crate::config::EnginesConfig;
use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

pub struct EngineRouter {
    engines: HashMap<EngineKind, Arc<dyn ComputeEngine>>,
    named_map: HashMap<String, EngineKind>,
    default_engine: EngineKind,
}

impl EngineRouter {
    pub fn new(default_engine: EngineKind) -> Self {
        Self {
            engines: HashMap::new(),
            named_map: HashMap::new(),
            default_engine,
        }
    }

    pub fn register(&mut self, engine: Arc<dyn ComputeEngine>) {
        let kind = engine.engine_kind();
        let name = kind.to_string();
        self.named_map.insert(name.to_lowercase(), kind.clone());
        self.engines.insert(kind, engine);
    }

    pub fn register_named(&mut self, name: &str, engine: Arc<dyn ComputeEngine>) {
        let kind = engine.engine_kind();
        self.named_map.insert(name.to_lowercase(), kind.clone());
        self.engines.insert(kind, engine);
    }

    /// Construct and configure an EngineRouter from an EnginesConfig instance
    pub fn from_config(config: &EnginesConfig) -> Self {
        let default_kind = EngineKind::from_name(&config.defaults.engine);
        let mut router = Self::new(default_kind);

        for (name, single) in &config.engines {
            match single.kind.as_str() {
                "antigravity" | "agy" => {
                    let engine = Arc::new(AntigravityEngine::new(
                        single.binary_path.clone(),
                        single.model.clone(),
                    ));
                    router.register_named(name, engine);
                }
                "claude_code" | "claude" => {
                    let engine = Arc::new(ClaudeCodeEngine::new(
                        single.binary_path.clone(),
                        single.model.clone(),
                    ));
                    router.register_named(name, engine);
                }
                _ => {
                    let binary = single.binary_path.clone().unwrap_or_else(|| name.clone());
                    let engine = Arc::new(GenericCliEngine::new(
                        name.clone(),
                        binary,
                        single.args.clone(),
                        single.env.clone(),
                        single.output_parser.clone(),
                    ));
                    router.register_named(name, engine);
                }
            }
        }

        info!(
            "Configured EngineRouter with {} engines",
            router.engines.len()
        );
        router
    }

    pub fn resolve_kind(&self, name_or_alias: Option<&str>) -> EngineKind {
        if let Some(alias) = name_or_alias {
            let key = alias.to_lowercase();
            if let Some(kind) = self.named_map.get(&key) {
                return kind.clone();
            }
            EngineKind::from_name(alias)
        } else {
            self.default_engine.clone()
        }
    }

    pub async fn dispatch(
        &self,
        preferred: Option<EngineKind>,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        let kind = preferred.unwrap_or_else(|| self.default_engine.clone());
        if let Some(engine) = self.engines.get(&kind) {
            engine.execute(input).await
        } else {
            Err(EngineError::CliNotFound(format!(
                "No registered engine for {:?}",
                kind
            )))
        }
    }

    pub async fn dispatch_by_name(
        &self,
        engine_name: Option<&str>,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        let kind = self.resolve_kind(engine_name);
        self.dispatch(Some(kind), input).await
    }
}
