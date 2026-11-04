import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { Link } from '../components/Layout';
import { GLOSSARY, isGlossaryKey, type GlossaryKey } from './glossaryData';
import './glossary.css';
import { GLOSSARY_LINK_LABEL } from './glossaryWording';

/** The address of a term's entry on the Glossary page. */
export const glossaryPath = (key: GlossaryKey): string => `/glossary#${key}`;

/**
 * Words with a definition behind them. The words are a button: a press, touch or Enter or Space opens the definition and
 * the same again, or Escape, or a press elsewhere, closes it. Nothing depends on hover. The definition is in the page
 * only while open, and is linked from the button, so a screen reader reads it where it is. An unknown key shows the plain
 * words, so a wrong key can never hide text.
 */
export function Term({ term, children }: { term: GlossaryKey; children?: ReactNode }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const root = useRef<HTMLSpanElement>(null);
  const button = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPress = (e: Event) => {
      if (e.target instanceof Node && !root.current?.contains(e.target)) setOpen(false);
    };
    document.addEventListener('pointerdown', onPress);
    return () => document.removeEventListener('pointerdown', onPress);
  }, [open]);

  if (!isGlossaryKey(term)) return <>{children}</>;
  const entry = GLOSSARY[term];
  return (
    <span className="term" ref={root}>
      <button
        type="button"
        className="term-button"
        ref={button}
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => setOpen((o) => !o)}
        onKeyDown={(e) => {
          if (e.key !== 'Escape' || !open) return;
          e.stopPropagation();
          setOpen(false);
          button.current?.focus();
        }}
      >
        {children ?? entry.term}
      </button>
      {open && (
        <span className="term-pop" id={id} role="group" aria-label={entry.term}>
          {entry.definition} <Link to={glossaryPath(term)}>{GLOSSARY_LINK_LABEL}</Link>
        </span>
      )}
    </span>
  );
}
