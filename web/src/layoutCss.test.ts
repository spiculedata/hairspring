import { describe, expect, it } from 'vitest';
import { readSource } from './readSource';

const styles = readSource('styles.css');
const shell = readSource('shell/shell.css');
const rule = (css: string, selector: string): string => {
  const start = css.indexOf(`\n${selector} {`);
  if (start < 0) throw new Error(`no rule for ${selector}`);
  return css.slice(start, css.indexOf('}', start));
};

describe('content column', () => {
  it('gives main the width of its grid column, so auto margins cannot size it to its widest content', () => {
    const main = rule(styles, 'main');
    expect(main).toMatch(/width:\s*100%/);
    expect(main).toMatch(/min-width:\s*0/);
  });

  it('lets the grid column shrink below the width of its content', () => {
    expect(shell).toMatch(/grid-template-columns:\s*12\.5rem minmax\(0, 1fr\)/);
    expect(shell).toMatch(/@media \(max-width: 860px\)\s*\{\s*\.frame \{ grid-template-columns: minmax\(0, 1fr\)/);
  });

  it('keeps a long line of preformatted text inside its own scrolling box', () => {
    expect(rule(styles, 'pre')).toMatch(/overflow:\s*auto/);
    expect(rule(styles, 'pre')).toMatch(/max-width:\s*100%/);
  });
});
