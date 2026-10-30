use std::collections::HashMap;
use std::sync::Arc;
use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};

pub struct EngineRouter {
    engines: HashMap<EngineKind, Arc<dyn ComputeEngine>>,
    default_engine: EngineKind,
}

impl EngineRouter {
    pub fn new(default_engine: EngineKind) -> Self {
        Self {
            engines: HashMap::new(),
            default_engine,
        }
    }

    pub fn register(&mut self, engine: Arc<dyn ComputeEngine>) {
        self.engines.insert(engine.engine_kind(), engine);
    }

    pub async fn dispatch(
        &self,
        preferred: Option<EngineKind>,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        let kind = preferred.unwrap_or(self.default_engine.clone());
        if let Some(engine) = self.engines.get(&kind) {
            engine.execute(input).await
        } else {
            Err(EngineError::CliNotFound(format!(
                "No registered engine for {:?}",
                kind
            )))
        }
    }
}
