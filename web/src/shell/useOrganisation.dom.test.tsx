// @vitest-environment happy-dom
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, setToken } from '../api';
import { useOrganisation } from './useOrganisation';

function Probe() {
  return <p data-testid="name">{useOrganisation() ?? 'none'}</p>;
}

const answer = (organisation?: unknown) => ({ status: 'ok', version: '0.1.0', ...(organisation === undefined ? {} : { organisation }) });
const name = () => screen.getByTestId('name').textContent;

describe('useOrganisation', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    setToken(null);
  });

  it('is nothing until the server answers, and when it names no organisation', async () => {
    const health = vi.spyOn(api, 'health').mockResolvedValue(answer());
    render(<Probe />);
    expect(name()).toBe('none');
    await waitFor(() => expect(health).toHaveBeenCalledTimes(1));
    expect(name()).toBe('none');
  });

  it('is the name the server sends', async () => {
    vi.spyOn(api, 'health').mockResolvedValue(answer('Acme Ltd'));
    render(<Probe />);
    await waitFor(() => expect(name()).toBe('Acme Ltd'));
  });

  it('ignores a name that is not valid', async () => {
    const health = vi.spyOn(api, 'health').mockResolvedValue(answer('a‮b'));
    render(<Probe />);
    await waitFor(() => expect(health).toHaveBeenCalled());
    expect(name()).toBe('none');
  });

  it('shows no name, and no error, when the request fails', async () => {
    const health = vi.spyOn(api, 'health').mockRejectedValue(new Error('network down'));
    render(<Probe />);
    await waitFor(() => expect(health).toHaveBeenCalled());
    expect(name()).toBe('none');
  });

  it('asks again when a token is entered, because the name goes only to a caller with a valid token', async () => {
    const health = vi.spyOn(api, 'health').mockResolvedValueOnce(answer()).mockResolvedValue(answer('Acme Ltd'));
    render(<Probe />);
    await waitFor(() => expect(health).toHaveBeenCalledTimes(1));
    expect(name()).toBe('none');
    act(() => setToken('a-token-0123456789abcdef'));
    await waitFor(() => expect(name()).toBe('Acme Ltd'));
    expect(health).toHaveBeenCalledTimes(2);
  });

  it('keeps the name it has when a later request fails', async () => {
    const health = vi.spyOn(api, 'health').mockResolvedValueOnce(answer('Acme Ltd')).mockRejectedValue(new Error('offline'));
    render(<Probe />);
    await waitFor(() => expect(name()).toBe('Acme Ltd'));
    act(() => setToken('another-token-0123456789'));
    await waitFor(() => expect(health).toHaveBeenCalledTimes(2));
    expect(name()).toBe('Acme Ltd');
  });

  it('does not set state after it is unmounted', async () => {
    let resolve: (v: ReturnType<typeof answer>) => void = () => undefined;
    vi.spyOn(api, 'health').mockReturnValue(new Promise((r) => (resolve = r)));
    const errors = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const view = render(<Probe />);
    view.unmount();
    await act(async () => resolve(answer('Late Ltd')));
    expect(errors).not.toHaveBeenCalled();
  });
});
