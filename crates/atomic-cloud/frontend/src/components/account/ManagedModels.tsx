import { useState } from 'react';
import type { FormEvent } from 'react';
import { Select } from '../ui/Select';
import { Button } from '../ui/Button';
import { Banner } from '../ui/Banner';
import { ApiError, updateModels } from '../../lib/api';
import type { ProviderWriteResult } from '../../lib/api';
import { MANAGED_EMBEDDING_MODEL, MANAGED_LLM_MODELS } from '../../lib/models';

interface ManagedModelsProps {
  /** The current `llm_model` from the stored managed config, if any. */
  currentLlmModel: string | null;
  onSaved: (result: ProviderWriteResult) => void;
}

/**
 * Model selection for the managed provider: the embedding model is platform-
 * pinned (shown read-only — changing it would invalidate every stored vector),
 * and the LLM is chosen from the curated list. Submits only the user-writable
 * `llm_model` to `PUT /api/account/provider/models`; the server merges it over
 * the platform-owned config.
 */
export function ManagedModels({ currentLlmModel, onSaved }: ManagedModelsProps) {
  const initial = currentLlmModel ?? MANAGED_LLM_MODELS[0].id;
  const [llmModel, setLlmModel] = useState(initial);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const changed = llmModel !== (currentLlmModel ?? initial);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!changed || submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      const result = await updateModels({ llm_model: llmModel });
      onSaved(result);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : 'Couldn’t update the model. Try again.');
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-5">
      <div className="rounded-xl border border-border-light bg-bg-secondary/40 px-4 py-3">
        <p className="text-sm font-medium text-text-primary">Embedding model</p>
        <p className="mt-0.5 font-mono text-sm text-text-secondary">{MANAGED_EMBEDDING_MODEL}</p>
        <p className="mt-1 text-xs text-text-muted">
          Pinned on the managed plan — changing it would invalidate every stored
          embedding. Bring your own key to choose a different one.
        </p>
      </div>

      {error && (
        <Banner tone="error" title="Couldn’t update the model">
          {error}
        </Banner>
      )}

      <Select
        label="LLM model"
        value={llmModel}
        onChange={(e) => setLlmModel(e.target.value)}
        disabled={submitting}
        options={MANAGED_LLM_MODELS.map((m) => ({ value: m.id, label: m.label }))}
        help="Used for tagging, wiki synthesis, and chat."
      />

      <Button type="submit" disabled={!changed} loading={submitting}>
        {submitting ? 'Saving…' : 'Save model'}
      </Button>
    </form>
  );
}
