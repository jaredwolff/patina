use std::collections::HashMap;

#[allow(deprecated)]
use rig::client::completion::CompletionModelHandle;

/// Pool of named model tiers (e.g. "default", "coding", "consolidation").
///
/// Unknown tier names fall back to "default". The "default" tier must always
/// be present — this is validated at construction time.
#[allow(deprecated)]
#[derive(Clone)]
pub struct ModelPool {
    /// (model_handle, model_name_for_logging)
    models: HashMap<String, (CompletionModelHandle<'static>, String)>,
}

#[allow(deprecated)]
impl ModelPool {
    /// Create a new ModelPool. Panics if no "default" tier is present.
    pub fn new(models: HashMap<String, (CompletionModelHandle<'static>, String)>) -> Self {
        assert!(
            models.contains_key("default"),
            "ModelPool must contain a \"default\" tier"
        );
        Self { models }
    }

    /// Get a specific tier. Falls back to "default" if the tier is not found.
    pub fn get(&self, tier: &str) -> (&CompletionModelHandle<'static>, &str) {
        let (handle, name) = self
            .models
            .get(tier)
            .or_else(|| self.models.get("default"))
            .expect("default tier must exist");
        (handle, name)
    }

    /// Get the default tier.
    pub fn default_model(&self) -> (&CompletionModelHandle<'static>, &str) {
        self.get("default")
    }

    /// List all available tier names.
    pub fn tiers(&self) -> Vec<&str> {
        self.models.keys().map(|k| k.as_str()).collect()
    }
}
