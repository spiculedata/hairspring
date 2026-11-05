// @vitest-environment happy-dom
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { navigate } from '../router';
import { runFiltersFromSearch, runFiltersToSearch, type RunFilters } from './filters';
import { useAddressFilters } from './useAddressFilters';

const setup = () => renderHook(() => useAddressFilters<RunFilters>(runFiltersFromSearch, runFiltersToSearch));
const address = (): string => window.location.pathname + window.location.search;

describe('useAddressFilters', () => {
  beforeEach(() => window.history.replaceState({}, '', '/runs'));
  afterEach(() => {
    cleanup();
    window.history.replaceState({}, '', '/');
  });

  it('starts from the filters in the address, leaving out anything invalid, and tidies the address to match', () => {
    window.history.replaceState({}, '', '/runs?state=failed&mode=nope&junk=1');
    const { result } = setup();
    expect(result.current[0]).toEqual({ state: 'failed' });
    expect(address()).toBe('/runs?state=failed');
  });

  it('has no filters for a bare address, and leaves it as it is', () => {
    const { result } = setup();
    expect(result.current[0]).toEqual({});
    expect(address()).toBe('/runs');
  });

  it('writes the filters to the address in place, adding no history entry', () => {
    const before = window.history.length;
    const { result } = setup();
    act(() => result.current[1]({ state: 'running', target: 'a b' }));
    expect(address()).toBe('/runs?state=running&target=a+b');
    expect(window.history.length).toBe(before);
    act(() => result.current[1]({}));
    expect(address()).toBe('/runs');
  });

  it('takes the filters of a link followed in the page, such as the Runs link in the navigation', () => {
    const { result } = setup();
    act(() => result.current[1]({ state: 'failed' }));
    act(() => navigate('/runs'));
    expect(result.current[0]).toEqual({});
    act(() => navigate('/runs?mode=sweep'));
    expect(result.current[0]).toEqual({ mode: 'sweep' });
  });

  it('takes the filters again when the reader goes back', () => {
    const { result } = setup();
    act(() => navigate('/runs?state=queued'));
    act(() => window.dispatchEvent(new PopStateEvent('popstate')));
    expect(result.current[0]).toEqual({ state: 'queued' });
  });

  it('keeps the same filters object when a change of address means no change of filters', () => {
    const { result } = setup();
    act(() => result.current[1]({ state: 'failed' }));
    const held = result.current[0];
    act(() => navigate('/runs?state=failed'));
    expect(result.current[0]).toBe(held);
  });

  it('leaves the address of another page alone when the reader goes there', () => {
    const { result } = setup();
    act(() => result.current[1]({ state: 'failed' }));
    act(() => navigate('/failures?status=verified'));
    expect(address()).toBe('/failures?status=verified');
    expect(result.current[0]).toEqual({ state: 'failed' });
  });

  it('stops listening when the page is left', () => {
    const { result, unmount } = setup();
    unmount();
    act(() => navigate('/runs?state=failed'));
    expect(result.current[0]).toEqual({});
  });
});
