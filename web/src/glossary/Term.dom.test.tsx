// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { GLOSSARY } from './glossaryData';
import { GlossaryPage } from './GlossaryPage';
import { Term } from './Term';

afterEach(cleanup);

describe('Term', () => {
  it('shows its words as a button that is closed to begin with, and the definition is not in the page yet', () => {
    render(<Term term="seed">the seed</Term>);
    const button = screen.getByRole('button', { name: 'the seed' });
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByText(GLOSSARY.seed.definition)).toBeNull();
  });

  it('uses the glossary term when given no words', () => {
    render(<Term term="bundle" />);
    expect(screen.getByRole('button', { name: 'Bundle' })).toBeTruthy();
  });

  it('opens on a click and closes on a second click, so it works by touch and keyboard, not by hover', () => {
    render(<Term term="seed">seed</Term>);
    const button = screen.getByRole('button', { name: 'seed' });
    fireEvent.click(button);
    expect(button.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByText(GLOSSARY.seed.definition)).toBeTruthy();
    fireEvent.click(button);
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByText(GLOSSARY.seed.definition)).toBeNull();
  });

  it('points the button at the definition it shows', () => {
    render(<Term term="seed">seed</Term>);
    const button = screen.getByRole('button', { name: 'seed' });
    fireEvent.click(button);
    const id = button.getAttribute('aria-controls');
    expect(id).toBeTruthy();
    expect(document.getElementById(id as string)?.textContent).toContain(GLOSSARY.seed.definition);
  });

  it('closes on Escape and keeps focus on the button', () => {
    render(<Term term="seed">seed</Term>);
    const button = screen.getByRole('button', { name: 'seed' });
    button.focus();
    fireEvent.click(button);
    fireEvent.keyDown(button, { key: 'Escape' });
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(button);
  });

  it('closes when the reader presses elsewhere', () => {
    render(
      <>
        <Term term="seed">seed</Term>
        <p>elsewhere</p>
      </>,
    );
    const button = screen.getByRole('button', { name: 'seed' });
    fireEvent.click(button);
    fireEvent.pointerDown(screen.getByText('elsewhere'));
    expect(button.getAttribute('aria-expanded')).toBe('false');
  });

  it('links to the entry on the glossary page from the open definition', () => {
    render(<Term term="seed">seed</Term>);
    fireEvent.click(screen.getByRole('button', { name: 'seed' }));
    const link = screen.getByRole('link', { name: /glossary/i });
    expect(link.getAttribute('href')).toBe('/glossary#seed');
  });

  it('shows only the words when the key is not in the glossary, never an empty definition', () => {
    // @ts-expect-error a key outside the glossary is a type error, and still must not break the page
    render(<Term term="nonsense">plain words</Term>);
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.getByText('plain words')).toBeTruthy();
  });
});

describe('GlossaryPage', () => {
  it('has one heading and lists every entry as a term with its definition and an address fragment', () => {
    const { container } = render(<GlossaryPage />);
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
    for (const [key, e] of Object.entries(GLOSSARY)) {
      const dt = container.querySelector(`dt#${key}`);
      expect(dt?.textContent, key).toBe(e.term);
      expect(dt?.nextElementSibling?.textContent, key).toBe(e.definition);
    }
  });
});
