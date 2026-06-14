import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ByokForm } from './ByokForm';
import * as apiModule from '../../lib/api';
import { ApiError } from '../../lib/api';

describe('ByokForm', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('disables submit until the form is minimally valid', async () => {
    const user = userEvent.setup();
    render(<ByokForm hasExistingKey={false} onSaved={() => {}} />);

    const submit = screen.getByRole('button', { name: /save & validate/i });
    // No key yet → disabled.
    expect(submit).toBeDisabled();

    // A non-empty key on the default OpenRouter provider is enough.
    await user.type(screen.getByLabelText(/api key/i), 'sk-or-test');
    expect(submit).toBeEnabled();
  });

  it('requires a base URL for the OpenAI-compatible provider', async () => {
    const user = userEvent.setup();
    render(<ByokForm hasExistingKey={false} onSaved={() => {}} />);

    // Switch to OpenAI-compatible (a radio in the segmented control).
    await user.click(screen.getByRole('radio', { name: /openai-compatible/i }));

    const submit = screen.getByRole('button', { name: /save & validate/i });
    await user.type(screen.getByLabelText(/api key/i), 'sk-test');
    // A key alone isn't enough for openai_compat — the base URL is required.
    expect(submit).toBeDisabled();

    await user.type(screen.getByLabelText(/base url/i), 'https://endpoint.example/v1');
    expect(submit).toBeEnabled();
  });

  it('surfaces the server validation error verbatim and stores nothing', async () => {
    const user = userEvent.setup();
    const providerMessage = 'HTTP 401: invalid api key for provider';
    const spy = vi
      .spyOn(apiModule, 'saveByokProvider')
      .mockRejectedValue(
        new ApiError(400, 'provider_validation_failed', providerMessage),
      );
    const onSaved = vi.fn();
    render(<ByokForm hasExistingKey={false} onSaved={onSaved} />);

    await user.type(screen.getByLabelText(/api key/i), 'sk-or-bad');
    await user.click(screen.getByRole('button', { name: /save & validate/i }));

    // The provider's message is shown verbatim.
    await waitFor(() => {
      expect(screen.getByText(providerMessage)).toBeInTheDocument();
    });
    expect(spy).toHaveBeenCalledTimes(1);
    // Nothing "stored": onSaved (the success path) never fired.
    expect(onSaved).not.toHaveBeenCalled();
  });

  it('never renders a key value — the input is a password and clears on success', async () => {
    const user = userEvent.setup();
    vi.spyOn(apiModule, 'saveByokProvider').mockResolvedValue({
      status: 'saved',
      provider: 'openrouter',
      origin: 'user',
      reembed_warning: null,
    });
    render(<ByokForm hasExistingKey onSaved={() => {}} />);

    const keyInput = screen.getByLabelText(/new api key/i) as HTMLInputElement;
    // The key field masks by default (never shows the secret as text).
    expect(keyInput.type).toBe('password');

    await user.type(keyInput, 'sk-or-secret-value');
    await user.click(screen.getByRole('button', { name: /replace key/i }));

    // On success the typed key is cleared from the field (not retained or
    // echoed back anywhere).
    await waitFor(() => {
      expect((screen.getByLabelText(/new api key/i) as HTMLInputElement).value).toBe('');
    });
    // And the secret never appears as visible text anywhere in the document.
    expect(screen.queryByText('sk-or-secret-value')).not.toBeInTheDocument();
  });
});
