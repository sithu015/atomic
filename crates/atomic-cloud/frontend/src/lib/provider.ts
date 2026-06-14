/** Shared provider-display helpers for the overview and provider pages. */

import type { ByokModelConfig } from './api';

/** Human label for a provider id. */
export function providerLabel(provider: string | null | undefined): string {
  switch (provider) {
    case 'openrouter':
      return 'OpenRouter';
    case 'openai_compat':
      return 'OpenAI-compatible';
    default:
      return 'Not configured';
  }
}

/** Human label for a credential origin. */
export function originLabel(origin: string | null | undefined): string {
  switch (origin) {
    case 'managed':
      return 'Managed by Atomic';
    case 'user':
      return 'Your own key';
    default:
      return 'None';
  }
}

/**
 * Read the typed BYOK fields out of an untyped `model_config` bag (the server
 * stores plaintext JSON). Only the documented BYOK vocabulary is surfaced —
 * an unexpected key (which the server would have rejected on write) is ignored.
 */
export function readModelConfig(
  config: Record<string, unknown> | null | undefined,
): ByokModelConfig {
  if (!config) return {};
  const str = (k: string): string | undefined =>
    typeof config[k] === 'string' ? (config[k] as string) : undefined;
  const num = (k: string): number | undefined =>
    typeof config[k] === 'number' ? (config[k] as number) : undefined;
  return {
    embedding_model: str('embedding_model'),
    llm_model: str('llm_model'),
    openrouter_base_url: str('openrouter_base_url'),
    openai_compat_base_url: str('openai_compat_base_url'),
    embedding_dimension: num('embedding_dimension'),
  };
}
