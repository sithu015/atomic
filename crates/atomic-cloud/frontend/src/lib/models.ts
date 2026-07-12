/**
 * The curated managed-model catalogue, mirrored from the server's
 * `crate::curated_models`. The server is the authority — these constants drive
 * the managed model picker's options and the dimension hint; an out-of-list or
 * out-of-tier choice is rejected server-side (`model_not_curated`).
 */

/** The platform-pinned embedding model for managed keys (server:
 * `MANAGED_EMBEDDING_MODEL`). Fixed — changing it would invalidate every
 * stored vector, so the managed picker shows it read-only. */
export const MANAGED_EMBEDDING_MODEL = 'qwen/qwen3-embedding-8b';

/** The platform-pinned embedding dimension (server:
 * `PINNED_EMBEDDING_DIMENSION`). A BYOK config whose effective dimension
 * differs is rejected server-side; the BYOK form surfaces that. */
export const PINNED_EMBEDDING_DIMENSION = 1536;

/** The fixed managed tagging model (server: `MANAGED_TAGGING_MODEL`). Tagging
 * is a single-shot utility task — platform-owned and not user-selectable, so
 * the picker shows it read-only. Wiki/chat/reports use the agentic model. */
export const MANAGED_TAGGING_MODEL = 'openai/gpt-5-nano';

export interface ManagedModel {
  id: string;
  label: string;
}

/**
 * The agentic model lists (wiki, chat, reports) by tier, mirrored from the
 * server's `FREE_AGENTIC_MODELS` / `PRO_AGENTIC_MODELS`. The first free entry
 * is the signup default.
 *
 * TODO(DASH-2): hand-maintained mirror with no drift guard. The server is the
 * authority — an out-of-list/out-of-tier choice is rejected with
 * `model_not_curated` — so keep these in lockstep with `crate::curated_models`
 * when editing either side, or close the drift by exposing the catalogue on the
 * provider response.
 */
export const FREE_AGENTIC_MODELS: ReadonlyArray<ManagedModel> = [
  { id: 'openai/gpt-5-mini', label: 'GPT-5 mini' },
  { id: 'google/gemini-3.1-flash-lite', label: 'Gemini 3.1 Flash-Lite' },
];

export const PRO_AGENTIC_MODELS: ReadonlyArray<ManagedModel> = [
  { id: 'openai/gpt-5-mini', label: 'GPT-5 mini' },
  { id: 'google/gemini-3.1-flash-lite', label: 'Gemini 3.1 Flash-Lite' },
  { id: 'anthropic/claude-sonnet-5', label: 'Claude Sonnet 5' },
  { id: 'openai/gpt-5.6-terra', label: 'GPT-5.6 Terra' },
  { id: 'z-ai/glm-5.2', label: 'GLM-5.2' },
];

/** The agentic model list a plan may pick from (server:
 * `agentic_models_for_plan`). Premium plans unlock the fuller set. */
export function managedAgenticModels(premium: boolean): ReadonlyArray<ManagedModel> {
  return premium ? PRO_AGENTIC_MODELS : FREE_AGENTIC_MODELS;
}

/** Human label for a managed agentic id, falling back to the raw id. Searches
 * the premium superset so every valid id resolves regardless of tier. */
export function managedLlmLabel(id: string): string {
  return PRO_AGENTIC_MODELS.find((m) => m.id === id)?.label ?? id;
}
