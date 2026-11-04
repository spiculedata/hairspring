// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { navigate } from '../router';
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

  it('closes on Escape from the button and keeps focus on it', () => {
    render(<Term term="seed">seed</Term>);
    const button = screen.getByRole('button', { name: 'seed' });
    button.focus();
    fireEvent.click(button);
    fireEvent.keyDown(button, { key: 'Escape' });
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(button);
  });

  it('closes on Escape from the Glossary link inside the definition and returns focus to the button', () => {
    render(<Term term="seed">seed</Term>);
    const button = screen.getByRole('button', { name: 'seed' });
    fireEvent.click(button);
    const link = screen.getByRole('link', { name: /glossary/i });
    link.focus();
    expect(document.activeElement).toBe(link);
    fireEvent.keyDown(link, { key: 'Escape' });
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByText(GLOSSARY.seed.definition)).toBeNull();
    expect(document.activeElement).toBe(button);
  });

  it('ignores Escape while closed, so it is left to whatever else is listening', () => {
    const onKeyDown = vi.fn();
    render(
      <div onKeyDown={onKeyDown}>
        <Term term="seed">seed</Term>
      </div>,
    );
    const button = screen.getByRole('button', { name: 'seed' });
    button.focus();
    fireEvent.keyDown(button, { key: 'Escape' });
    expect(onKeyDown).toHaveBeenCalledTimes(1);
  });

  it('closes when keyboard focus leaves it, and stays open while focus moves inside it', () => {
    render(
      <>
        <Term term="seed">seed</Term>
        <a href="#after">after</a>
      </>,
    );
    const button = screen.getByRole('button', { name: 'seed' });
    fireEvent.click(button);
    const link = screen.getByRole('link', { name: /glossary/i });
    fireEvent.blur(button, { relatedTarget: link });
    fireEvent.focusOut(button, { relatedTarget: link });
    expect(button.getAttribute('aria-expanded')).toBe('true');
    const after = screen.getByRole('link', { name: 'after' });
    fireEvent.focusOut(link, { relatedTarget: after });
    expect(button.getAttribute('aria-expanded')).toBe('false');
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
  it('scrolls to the entry named by the address on mount and again when only the fragment changes', () => {
    const scroll = vi.fn();
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function (this: Element) {
      scroll(this.id);
    };
    try {
      act(() => navigate('/glossary#seed'));
      render(<GlossaryPage />);
      expect(scroll).toHaveBeenLastCalledWith('seed');
      act(() => navigate('/glossary#bundle'));
      expect(scroll).toHaveBeenLastCalledWith('bundle');
    } finally {
      Element.prototype.scrollIntoView = original;
      act(() => navigate('/'));
    }
  });

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
