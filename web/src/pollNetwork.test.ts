import { describe, expect, it } from 'vitest';
import { ApiError } from './api';
import { isNetworkFailure } from './pollPolicy';

describe('isNetworkFailure', () => {
  it('is true for the TypeError fetch throws when no response arrived', () => {
    expect(isNetworkFailure(new TypeError('Failed to fetch'))).toBe(true);
  });

  it('is false for any answer from the server, including a 500', () => {
    expect(isNetworkFailure(new ApiError(500, 'HTTP 500'))).toBe(false);
    expect(isNetworkFailure(new ApiError(502, 'bad shape'))).toBe(false);
  });

  it('is false for other errors, such as a body that is not JSON', () => {
    expect(isNetworkFailure(new SyntaxError('Unexpected token'))).toBe(false);
    expect(isNetworkFailure('Failed to fetch')).toBe(false);
  });
});
