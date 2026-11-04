import { useEffect } from 'react';
import { useHash } from '../router';
import { glossaryEntries } from './glossaryData';
import './glossary.css';
import { GLOSSARY_HEADING, GLOSSARY_INTRO } from './glossaryWording';

/** Brings the entry named by the address fragment into view: a pushed address does not scroll by itself, on mount and on each later change of the fragment. */
function scrollToFragment(hash: string): void {
  const key = decodeURIComponent(hash.slice(1));
  if (key === '') return;
  document.getElementById(key)?.scrollIntoView?.();
}

/** Every defined term, once, with its definition. Reached from a `Term`'s definition and from the Overview page. */
export function GlossaryPage() {
  const hash = useHash();
  useEffect(() => scrollToFragment(hash), [hash]);
  return (
    <>
      <h1>{GLOSSARY_HEADING}</h1>
      <p className="muted">{GLOSSARY_INTRO}</p>
      <dl className="glossary">
        {glossaryEntries().map((e) => (
          <div key={e.key} className="glossary-entry">
            <dt id={e.key}>{e.term}</dt>
            <dd>{e.definition}</dd>
          </div>
        ))}
      </dl>
    </>
  );
}
