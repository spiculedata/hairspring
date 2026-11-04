import { useEffect } from 'react';
import { glossaryEntries } from './glossaryData';
import './glossary.css';
import { GLOSSARY_HEADING, GLOSSARY_INTRO } from './glossaryWording';

/** Brings the entry named by the address fragment into view: a pushed address does not scroll by itself. */
function scrollToFragment(): void {
  const key = decodeURIComponent(window.location.hash.slice(1));
  if (key === '') return;
  document.getElementById(key)?.scrollIntoView?.();
}

/** Every defined term, once, with its definition. Reached from a `Term`'s definition and from the Overview page. */
export function GlossaryPage() {
  useEffect(scrollToFragment, []);
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
