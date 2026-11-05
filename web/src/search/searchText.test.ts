import { describe, expect, it } from 'vitest';
import { checkSearchText, MAX_SEARCH_CHARS, MIN_SEARCH_CHARS } from './searchText';

describe('checkSearchText', () => {
  it('has nothing to ask for until something is typed, and trims the spaces around a pasted id', () => {
    expect(checkSearchText('')).toEqual({ kind: 'empty' });
    expect(checkSearchText('   ')).toEqual({ kind: 'empty' });
    expect(checkSearchText('  run_8f3a  ')).toEqual({ kind: 'ok', text: 'run_8f3a' });
  });

  it('asks for a little more before it searches, since one character matches almost everything', () => {
    expect(checkSearchText('r')).toEqual({ kind: 'short' });
    expect(checkSearchText('ru')).toEqual({ kind: 'ok', text: 'ru' });
    expect(MIN_SEARCH_CHARS).toBe(2);
  });

  it('accepts exactly what the server accepts: ASCII letters, digits and _ - . : /, at most 64', () => {
    expect(checkSearchText('aZ09_-.:/')).toEqual({ kind: 'ok', text: 'aZ09_-.:/' });
    expect(checkSearchText('a'.repeat(MAX_SEARCH_CHARS))).toEqual({ kind: 'ok', text: 'a'.repeat(MAX_SEARCH_CHARS) });
    expect(checkSearchText('a'.repeat(MAX_SEARCH_CHARS + 1))).toEqual({ kind: 'invalid', problem: 'long' });
  });

  it('refuses anything else without sending it: spaces inside, wildcards, quotes, markup, control and non-ASCII characters', () => {
    for (const text of ['a b', 'a%', 'a*', "x' OR 1=1", '<b>', 'café', 'a\nb', 'a\u0000b', '‮run_', 'a\\b', 'a?b', 'a&b=c', 'a#b']) {
      expect(checkSearchText(text), JSON.stringify(text)).toEqual({ kind: 'invalid', problem: 'characters' });
    }
  });

  it('counts characters, not UTF-16 units, so a long non-ASCII text is refused for its characters first', () => {
    expect(checkSearchText('\u{1F600}'.repeat(3))).toEqual({ kind: 'invalid', problem: 'characters' });
  });
});
