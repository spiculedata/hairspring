import { useEffect, useId, useRef, useState, type FocusEvent, type KeyboardEvent, type ReactNode, type RefObject } from 'react';
import { Link } from '../components/Layout';
import { GLOSSARY, isGlossaryKey, type GlossaryKey } from './glossaryData';
import './glossary.css';
import { GLOSSARY_LINK_LABEL } from './glossaryWording';

/** The address of a term's entry on the Glossary page. */
export const glossaryPath = (key: GlossaryKey): string => `/glossary#${key}`;

/**
 * Words with a definition behind them. The words are a button: a press, touch or Enter or Space opens the definition and
 * the same again, or Escape from anywhere inside it, or a press or keyboard focus outside it, closes it. Escape returns
 * focus to the button. Nothing depends on hover. The definition is in the page only while open, and is linked from the
 * button, so a screen reader reads it where it is. An unknown key shows the plain words, so a wrong key can never hide
 * text. With `heading`, the button sits inside a level 2 heading with that id and the definition follows the heading, not
 * inside it, so the heading's accessible name stays the words alone.
 */
export function Term({ term, children, heading }: { term: GlossaryKey; children?: ReactNode; heading?: { id: string } }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const root = useRef<HTMLElement>(null);
  const button = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPress = (e: Event) => {
      if (e.target instanceof Node && !root.current?.contains(e.target)) setOpen(false);
    };
    document.addEventListener('pointerdown', onPress);
    return () => document.removeEventListener('pointerdown', onPress);
  }, [open]);

  if (!isGlossaryKey(term)) return heading ? <h2 id={heading.id}>{children}</h2> : <>{children}</>;
  const entry = GLOSSARY[term];
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== 'Escape' || !open) return;
    e.stopPropagation();
    setOpen(false);
    button.current?.focus();
  };
  const onBlur = (e: FocusEvent) => {
    const next = e.relatedTarget;
    if (open && !(next instanceof Node && root.current?.contains(next))) setOpen(false);
  };
  const toggle = (
    <button
      type="button"
      className="term-button"
      ref={button}
      aria-expanded={open}
      aria-controls={open ? id : undefined}
      onClick={() => setOpen((o) => !o)}
    >
      {children ?? entry.term}
    </button>
  );
  const pop = open && (
    <span className="term-pop" id={id} role="group" aria-label={entry.term}>
      {entry.definition} <Link to={glossaryPath(term)}>{GLOSSARY_LINK_LABEL}</Link>
    </span>
  );
  if (heading) {
    return (
      <div className="term-block" ref={root as RefObject<HTMLDivElement>} onKeyDown={onKeyDown} onBlur={onBlur}>
        <h2 id={heading.id}>{toggle}</h2>
        {pop}
      </div>
    );
  }
  return (
    <span className="term" ref={root as RefObject<HTMLSpanElement>} onKeyDown={onKeyDown} onBlur={onBlur}>
      {toggle}
      {pop}
    </span>
  );
}
