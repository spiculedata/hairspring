import { describe, expect, it } from 'vitest';
import { readSource } from './readSource';

describe('the page language', () => {
  it('is British English, matching the copy and the en-GB time format', () => {
    expect(readSource('../index.html')).toMatch(/<html lang="en-GB">/);
  });
});
