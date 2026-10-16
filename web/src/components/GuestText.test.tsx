import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { CH } from '../guest/chars';
import { GuestText } from './GuestText';

describe('GuestText', () => {
  it('renders markup as escaped text, never as elements', () => {
    const html = renderToString(<GuestText text={'<script>alert(1)</script>'} />);
    expect(html).toContain('&lt;script&gt;alert(1)&lt;/script&gt;');
    expect(html).not.toContain('<script');
  });

  it('renders attributes from guest text as text', () => {
    const html = renderToString(<GuestText text={'<img src=x onerror=alert(1)>'} />);
    expect(html).not.toContain('<img');
    expect(html).toContain('&lt;img src=x onerror=alert(1)&gt;');
  });

  it('never makes a link out of a javascript: string', () => {
    const html = renderToString(<GuestText text={'javascript:alert(1)'} />);
    expect(html).not.toMatch(/<a[\s>]/);
    expect(html).not.toContain('href');
    expect(html).toContain('javascript:alert(1)');
  });

  it('shows a bidi override as a marked, visible escape', () => {
    const html = renderToString(<GuestText text={`a${CH.RLO}b`} />);
    expect(html).not.toContain(CH.RLO);
    expect(html).toContain('<span class="guest-esc">\\u{202E}</span>');
  });

  it('truncates with a visible marker and offers to show all', () => {
    const html = renderToString(<GuestText text={'x'.repeat(600)} />);
    expect(html).toContain('… (88 more characters)');
    expect(html).toContain('show all');
  });

  it('does not offer show-all where the row height is fixed', () => {
    const html = renderToString(<GuestText text={'x'.repeat(600)} max={100} expandable={false} />);
    expect(html).toContain('… (500 more characters)');
    expect(html).not.toContain('show all');
  });

  it('says where the text came from', () => {
    const html = renderToString(<GuestText text="hello" />);
    expect(html).toContain('from the program under test');
  });

  it('never emits raw attributes derived from the text', () => {
    const html = renderToString(<GuestText text={'" onmouseover="alert(1)'} />);
    expect(html).not.toContain('" onmouseover');
    expect(html).toContain('&quot; onmouseover=&quot;alert(1)');
  });

  it('survives a 10 MB string quickly and renders only the limit', () => {
    const big = '<'.repeat(10 * 1024 * 1024);
    const t0 = Date.now();
    const html = renderToString(<GuestText text={big} />);
    expect(Date.now() - t0).toBeLessThan(2000);
    expect(html.length).toBeLessThan(10_000);
    expect(html).toContain('more characters)');
  });
});
