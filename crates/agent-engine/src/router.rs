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
    default_name: Option<String>,
    configuration_errors: HashMap<String, String>,
    named_engines: HashMap<String, Arc<dyn ComputeEngine>>,
}

impl EngineRouter {
    pub fn new(default_engine: EngineKind) -> Self {
        Self {
            engines: HashMap::new(),
            named_map: HashMap::new(),
            default_engine,
            default_name: None,
            configuration_errors: HashMap::new(),
            named_engines: HashMap::new(),
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
        self.named_engines
            .insert(name.to_lowercase(), engine.clone());
        self.engines.entry(kind).or_insert(engine);
    }

    /// Construct and configure an EngineRouter from an EnginesConfig instance
    pub fn from_config(config: &EnginesConfig) -> Self {
        let default_kind = EngineKind::from_name(&config.defaults.engine);
        let mut router = Self::new(default_kind);

        router.default_name = Some(config.defaults.engine.clone());
        for name in config.engines.keys() {
            match configured_engine(config, name, None) {
                Ok(engine) => router.register_named(name, engine),
                Err(error) => {
                    router
                        .configuration_errors
                        .insert(name.to_lowercase(), error.to_string());
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
        if preferred.is_none() {
            if let Some(error) = self
                .default_name
                .as_ref()
                .and_then(|name| self.configuration_errors.get(&name.to_lowercase()))
            {
                return Err(EngineError::CliNotFound(error.clone()));
            }
            if let Some(engine) = self
                .default_name
                .as_ref()
                .and_then(|name| self.named_engines.get(&name.to_lowercase()))
            {
                return engine.execute(input).await;
            }
        }
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
        let name = engine_name.or(self.default_name.as_deref());
        if let Some(name) = name {
            if let Some(error) = self.configuration_errors.get(&name.to_lowercase()) {
                return Err(EngineError::CliNotFound(error.clone()));
            }
            if let Some(engine) = self.named_engines.get(&name.to_lowercase()) {
                return engine.execute(input).await;
            }
        }
        let kind = self.resolve_kind(name);
        self.dispatch(Some(kind), input).await
    }
}

/// One resolver shared by the CLI, REST server, workflow and Rig operator.
/// Unknown engines fail explicitly instead of executing a different provider.
pub fn configured_engine(
    config: &EnginesConfig,
    name: &str,
    model_override: Option<&str>,
) -> Result<Arc<dyn ComputeEngine>, EngineError> {
    let name = name.to_lowercase();
    let canonical = match name.as_str() {
        "claude_code" | "claude-code" | "claudecode" => "claude",
        "agy" => "antigravity",
        other => other,
    };
    let single = config
        .engines
        .get(&name)
        .or_else(|| config.engines.get(canonical));
    let kind = single
        .map(|engine| engine.kind.as_str())
        .unwrap_or(canonical);
    let binary = single.and_then(|engine| engine.binary_path.clone());
    let model = model_override
        .filter(|model| !model.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| single.and_then(|engine| engine.model.clone()));
    match kind {
        "codex" => Ok(Arc::new(crate::CodexEngine::new(binary, model).with_env(
            single.map(|engine| engine.env.clone()).unwrap_or_default(),
        ))),
        "claude" | "claude_code" | "claude-code" => {
            Ok(Arc::new(ClaudeCodeEngine::new(binary, model)))
        }
        "antigravity" | "agy" => Ok(Arc::new(AntigravityEngine::new(binary, model))),
        "local" | "omniroute" => Ok(Arc::new(crate::OpenAiCompatibleEngine::new(
            "http://127.0.0.1:20128/v1",
            "omniroute",
            model.as_deref().unwrap_or("local"),
        ))),
        "mock" => Ok(Arc::new(crate::MockEngine::new("Mock completed task"))),
        "generic" => {
            let single = single.ok_or_else(|| {
                EngineError::CliNotFound(format!("Engine {name} has no configuration"))
            })?;
            Ok(Arc::new(GenericCliEngine::new(
                name.clone(),
                binary.unwrap_or(name),
                single.args.clone(),
                single.env.clone(),
                single.output_parser.clone(),
            )))
        }
        _ => Err(EngineError::CliNotFound(format!(
            "Unknown engine or unsupported engine kind: {name} ({kind})"
        ))),
    }
}
