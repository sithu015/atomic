//! Curated model policy for managed provider keys (plan: "Provider
//! management" → "Model curation"; decision 2026-06-09).
//!
//! Managed keys spend the platform's money, so the platform picks the
//! models:
//!
//! - **The embedding model is pinned fleet-wide** ([`MANAGED_EMBEDDING_MODEL`]).
//!   Switching embedding models invalidates every stored vector and forces a
//!   full re-embed billed to the platform, so it is not user-changeable at
//!   all in managed mode.
//! - **Tagging/wiki/chat run on a short curated list** ([`MANAGED_LLM_MODELS`])
//!   of cost-effective models; users pick within the list.
//! - Frontier-model access is a paid-tier feature flag — that lands with the
//!   billing slice's `plans.feature_flags`, not here.
//!
//! [`validate_managed_model_config`] is the enforcement point for **user
//! writes** to a managed row's `model_config` (the provider-settings routes
//! in [`crate::tenant_plane`]). Platform-side writes — the seed config in
//! [`crate::managed_keys::ManagedKeyConfig`] — are deliberately not run
//! through it: the composition may legitimately point managed keys at a
//! proxy base URL (tests, gateways), which a *user* must never be able to do
//! (redirecting the platform-funded key to an attacker-controlled endpoint
//! would exfiltrate it; see the key-shaped rule below).
//!
//! BYOK rows are exempt from all of this — their key, their bill, any model
//! they like. The one BYOK guardrail is a loud re-embed warning when an
//! embedding-model change is saved (plan text), produced by the routes, not
//! here.

use serde_json::Value;

/// The fleet-wide pinned embedding model for managed keys. Matches
/// atomic-core's OpenRouter default so settings-mode (self-hosted) and
/// explicit-mode (cloud) deployments embed identically, and matches the
/// 1536-dimension column the tenant schema is reconciled to.
pub const MANAGED_EMBEDDING_MODEL: &str = "openai/text-embedding-3-small";

/// The curated LLM list for managed keys (tagging, wiki, chat): 2-3
/// cost-effective models, per the plan. The first entry is the default
/// seeded at signup ([`crate::managed_keys::default_managed_model_config`]).
pub const MANAGED_LLM_MODELS: &[&str] = &[
    "openai/gpt-4o-mini",
    "anthropic/claude-3.5-haiku",
    "google/gemini-2.0-flash-001",
];

/// The `model_config` keys a **user** may write on a managed row. Anything
/// else is rejected — most importantly the base-URL override keys
/// (`openrouter_base_url` / `openai_compat_base_url`, see
/// [`crate::provider_config`]): a user-supplied base URL on a managed key
/// would route the platform-funded credential to an arbitrary endpoint.
const MANAGED_ALLOWED_KEYS: &[&str] = &["embedding_model", "llm_model"];

/// Validate a user-submitted `model_config` for a managed credentials row.
///
/// Returns `Err(message)` describing the first violation — the message is
/// written for the 400 response body of the models route, naming the pinned
/// embedding model or the curated list so the caller can self-correct. The
/// rules, in check order:
///
/// 1. `model_config` must be a JSON object.
/// 2. Only [`MANAGED_ALLOWED_KEYS`] may appear (module docs: the base-URL
///    exfiltration rule).
/// 3. Present values must be strings.
/// 4. `embedding_model`, when present, must equal
///    [`MANAGED_EMBEDDING_MODEL`] (absent is fine — the config builder's
///    default *is* the pinned model).
/// 5. `llm_model`, when present, must be on [`MANAGED_LLM_MODELS`].
pub fn validate_managed_model_config(model_config: &Value) -> Result<(), String> {
    let Some(object) = model_config.as_object() else {
        return Err("model_config must be a JSON object".to_string());
    };

    for (key, value) in object {
        if !MANAGED_ALLOWED_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "model_config key {key:?} is not configurable on the managed provider; \
                 allowed keys: {MANAGED_ALLOWED_KEYS:?}. Bring your own key to configure \
                 providers freely."
            ));
        }
        if !value.is_string() {
            return Err(format!("model_config.{key} must be a string"));
        }
    }

    if let Some(embedding) = object.get("embedding_model").and_then(Value::as_str) {
        if embedding != MANAGED_EMBEDDING_MODEL {
            return Err(format!(
                "the managed embedding model is pinned to {MANAGED_EMBEDDING_MODEL:?} \
                 (changing it would invalidate every stored embedding); bring your own \
                 key to use a different embedding model"
            ));
        }
    }

    if let Some(llm) = object.get("llm_model").and_then(Value::as_str) {
        if !MANAGED_LLM_MODELS.contains(&llm) {
            return Err(format!(
                "{llm:?} is not on the managed model list {MANAGED_LLM_MODELS:?}; \
                 bring your own key to use other models"
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_managed_config_passes_curation() {
        // The signup seed must always satisfy the policy it is governed by.
        let seed = crate::managed_keys::default_managed_model_config();
        assert_eq!(validate_managed_model_config(&seed), Ok(()));
        assert_eq!(seed["embedding_model"], json!(MANAGED_EMBEDDING_MODEL));
        assert_eq!(seed["llm_model"], json!(MANAGED_LLM_MODELS[0]));
    }

    #[test]
    fn curated_llm_choices_pass() {
        for model in MANAGED_LLM_MODELS {
            let config = json!({ "llm_model": model });
            assert_eq!(validate_managed_model_config(&config), Ok(()), "{model}");
        }
        // Partial and empty objects are fine — absent keys fall back to
        // defaults that are themselves curated.
        assert_eq!(validate_managed_model_config(&json!({})), Ok(()));
    }

    #[test]
    fn uncurated_llm_is_rejected() {
        let config = json!({ "llm_model": "openai/o1-pro" });
        let err = validate_managed_model_config(&config).unwrap_err();
        assert!(err.contains("not on the managed model list"), "{err}");
    }

    #[test]
    fn embedding_model_is_pinned() {
        let ok = json!({ "embedding_model": MANAGED_EMBEDDING_MODEL });
        assert_eq!(validate_managed_model_config(&ok), Ok(()));

        let other = json!({ "embedding_model": "openai/text-embedding-3-large" });
        let err = validate_managed_model_config(&other).unwrap_err();
        assert!(err.contains("pinned"), "{err}");
    }

    #[test]
    fn base_url_overrides_are_rejected() {
        // The exfiltration rule: a user must never point the managed key at
        // their own endpoint.
        for key in ["openrouter_base_url", "openai_compat_base_url"] {
            let config = json!({ key: "https://attacker.example/api/v1" });
            let err = validate_managed_model_config(&config).unwrap_err();
            assert!(err.contains("not configurable"), "{key}: {err}");
        }
    }

    #[test]
    fn non_object_and_non_string_values_are_rejected() {
        assert!(validate_managed_model_config(&json!("gpt")).is_err());
        assert!(validate_managed_model_config(&json!(["a"])).is_err());
        let err = validate_managed_model_config(&json!({ "llm_model": 42 })).unwrap_err();
        assert!(err.contains("must be a string"), "{err}");
    }
}
