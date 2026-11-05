import { useId, useState, type KeyboardEvent } from 'react';
import { GuestText } from '../components/GuestText';
import { navigate } from '../router';
import { groupHits, hiddenCount, hitCount, scopeNote, type HitGroup, type HitRow } from './searchHits';
import { useSearchAvailable } from './useSearchAvailable';
import { useSearchResults, type SearchView } from './useSearchResults';
import { MAX_SEARCH_CHARS } from './searchText';
import { BAD_CHARACTERS, matchCount, MORE_NOTE, NO_MATCHES, RESULTS_LABEL, SEARCH_LABEL, SEARCH_PLACEHOLDER, SEARCHING, SOME_HIDDEN, TOO_LONG, TOO_SHORT } from './searchWording';

/** The text under the box for a view with no list of hits, and the line a screen reader hears for any view. */
function noteOf(view: SearchView, groups: readonly HitGroup[], hidden: number): string | undefined {
  switch (view.status) {
    case 'idle':
      return undefined;
    case 'hint':
      return view.problem === 'short' ? TOO_SHORT : view.problem === 'long' ? TOO_LONG : BAD_CHARACTERS;
    case 'busy':
      return view.previous === undefined ? SEARCHING : undefined;
    case 'error':
      return view.message;
    case 'done':
      return groups.length > 0 ? undefined : hidden > 0 ? SOME_HIDDEN : NO_MATCHES;
  }
}

function announcement(view: SearchView, groups: readonly HitGroup[], hidden: number): string {
  if (view.status === 'done' && groups.length > 0) return hidden > 0 ? `${matchCount(hitCount(groups))} ${SOME_HIDDEN}` : matchCount(hitCount(groups));
  if (view.status === 'busy') return SEARCHING;
  return noteOf(view, groups, hidden) ?? '';
}

function Option({ row, active, id, onChoose, onHover }: { row: HitRow; active: boolean; id: string; onChoose: (r: HitRow) => void; onHover: (i: number) => void }) {
  return (
    // Not focusable: the input keeps focus and names the active option with aria-activedescendant. Pressing the mouse must not move focus.
    <div
      id={id}
      role="option"
      aria-selected={active}
      className={active ? 'search-option search-option-active' : 'search-option'}
      onMouseDown={(e) => e.preventDefault()}
      onMouseMove={() => onHover(row.index)}
      onClick={() => onChoose(row)}
    >
      <span className="search-hit-main">{row.guestText !== undefined ? <GuestText text={row.guestText} max={80} expandable={false} /> : row.main}</span>
      <span className="search-hit-detail">{row.detail}</span>
    </div>
  );
}

/**
 * The global search box in the top bar. It is an editable combobox with a listbox of grouped hits: Down and Up move
 * through the hits, Enter opens the active one (or the first), Escape closes the list and a second Escape clears the
 * text. Everything it shows is a text node; a property id, which comes from the program under test, goes through
 * `GuestText`. It is not drawn for a server that has no search endpoint.
 */
export function SearchBox() {
  const [availability, markAbsent] = useSearchAvailable();
  const [text, setText] = useState('');
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const base = useId();
  const { view } = useSearchResults(text, markAbsent);

  if (availability !== 'available') return null;

  const results = view.status === 'done' ? view.results : view.status === 'busy' ? view.previous : undefined;
  const groups = results === undefined ? [] : groupHits(results);
  const hidden = results === undefined ? 0 : hiddenCount(results, groups);
  const rows = groups.flatMap((g) => g.rows);
  const active = rows.find((r) => r.index === activeIndex);
  const listId = `${base}-list`;
  const optionId = (r: HitRow): string => `${base}-hit-${r.index}`;
  const note = noteOf(view, groups, hidden);
  const showPopup = open && view.status !== 'idle';
  const showList = showPopup && rows.length > 0;

  const choose = (row: HitRow) => {
    setOpen(false);
    setText('');
    setActiveIndex(-1);
    navigate(row.path);
  };
  const move = (step: number) => {
    setOpen(true);
    if (rows.length === 0) return;
    const at = rows.findIndex((r) => r.index === activeIndex);
    const next = at === -1 ? (step > 0 ? 0 : rows.length - 1) : (at + step + rows.length) % rows.length;
    setActiveIndex(rows[next].index);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      move(e.key === 'ArrowDown' ? 1 : -1);
    } else if (e.key === 'Enter') {
      const row = active ?? rows[0];
      if (showList && row !== undefined) {
        e.preventDefault();
        choose(row);
      }
    } else if (e.key === 'Escape') {
      if (showPopup) setOpen(false);
      else setText('');
      setActiveIndex(-1);
      e.preventDefault();
    }
  };

  return (
    <div className="search" role="search" onBlur={() => setOpen(false)}>
      <label className="sr-only" htmlFor={`${base}-input`}>
        {SEARCH_LABEL}
      </label>
      <input
        id={`${base}-input`}
        className="search-input"
        type="text"
        role="combobox"
        aria-expanded={showList}
        aria-controls={showList ? listId : undefined}
        aria-autocomplete="list"
        aria-activedescendant={showList && active !== undefined ? optionId(active) : undefined}
        autoComplete="off"
        autoCapitalize="off"
        spellCheck={false}
        maxLength={MAX_SEARCH_CHARS * 2}
        placeholder={SEARCH_PLACEHOLDER}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          setActiveIndex(-1);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onKeyDown={onKeyDown}
      />
      <div className="sr-only" role="status" aria-live="polite">
        {announcement(view, groups, hidden)}
      </div>
      {showPopup && (
        <div className="search-popup" onMouseDown={(e) => e.preventDefault()}>
          {note !== undefined && <p className={view.status === 'done' ? 'search-note' : 'search-note search-note-plain'}>{note}</p>}
          {showList && (
            <div id={listId} role="listbox" aria-label={RESULTS_LABEL} aria-busy={view.status === 'busy'}>
              {groups.map((g) => (
                <div key={g.id} role="group" aria-labelledby={`${base}-group-${g.id}`} className="search-group">
                  <div id={`${base}-group-${g.id}`} role="presentation" className="search-group-title">
                    {g.title}
                    {g.more && <span className="search-more"> (more)</span>}
                  </div>
                  {g.rows.map((r) => (
                    <Option key={r.key} row={r} id={optionId(r)} active={r.index === activeIndex} onChoose={choose} onHover={setActiveIndex} />
                  ))}
                </div>
              ))}
            </div>
          )}
          {view.status === 'done' && groups.length > 0 && hidden > 0 && <p className="search-scope">{SOME_HIDDEN}</p>}
          {view.status === 'done' && groups.length === 0 && hidden === 0 && <p className="search-scope">{scopeNote(view.results.scope)}</p>}
          {view.status === 'done' && groups.some((g) => g.more) && <p className="search-scope">{MORE_NOTE}</p>}
        </div>
      )}
    </div>
  );
}
