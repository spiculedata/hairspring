// @vitest-environment happy-dom
import { cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { useDocumentTitle } from './useDocumentTitle';

describe('useDocumentTitle', () => {
  afterEach(cleanup);

  it('sets the document title, and changes it when the title changes', () => {
    const { rerender } = renderHook(({ t }) => useDocumentTitle(t), { initialProps: { t: 'Runs · Hairspring' } });
    expect(document.title).toBe('Runs · Hairspring');
    rerender({ t: 'Run run_12 · Hairspring' });
    expect(document.title).toBe('Run run_12 · Hairspring');
  });
});
