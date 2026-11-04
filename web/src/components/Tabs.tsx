import { useRef, type KeyboardEvent, type ReactNode } from 'react';

export interface TabSpec {
  id: string;
  label: string;
  /** Shown after the label and read as part of the tab's name. */
  count?: number;
}

export interface TabsProps {
  /** Names the tab list for assistive technology. */
  label: string;
  /** Keeps the ids of this tab list distinct from any other on the page. */
  idPrefix: string;
  tabs: readonly TabSpec[];
  selected: string;
  onSelect: (id: string) => void;
  /** The selected tab's content. Only that panel is rendered, so a hidden panel does no work. */
  children: ReactNode;
}

/** Where an arrow, Home or End key leads from `index`, or undefined for any other key. */
export function targetIndex(key: string, index: number, count: number): number | undefined {
  if (key === 'ArrowRight') return (index + 1) % count;
  if (key === 'ArrowLeft') return (index - 1 + count) % count;
  if (key === 'Home') return 0;
  if (key === 'End') return count - 1;
  return undefined;
}

/**
 * Tabs by the WAI-ARIA tabs pattern with automatic activation: one tab is in the tab order, the arrow keys, Home and
 * End move selection and focus, and Tab goes on into the panel. The caller owns the selection (for example in the
 * address); this component only reports a choice through `onSelect`.
 */
export function Tabs({ label, idPrefix, tabs, selected, onSelect, children }: TabsProps) {
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const tabId = (id: string) => `${idPrefix}-tab-${id}`;

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = tabs.findIndex((t) => t.id === selected);
    const next = index < 0 ? undefined : targetIndex(event.key, index, tabs.length);
    if (next === undefined) return;
    event.preventDefault();
    const id = tabs[next].id;
    buttons.current.get(id)?.focus();
    onSelect(id);
  };

  return (
    <>
      <div className="tabs" role="tablist" aria-label={label} onKeyDown={onKeyDown}>
        {tabs.map((t) => (
          <button
            key={t.id}
            ref={(el) => {
              if (el) buttons.current.set(t.id, el);
              else buttons.current.delete(t.id);
            }}
            type="button"
            role="tab"
            id={tabId(t.id)}
            aria-controls={`${idPrefix}-panel`}
            aria-selected={t.id === selected}
            tabIndex={t.id === selected ? 0 : -1}
            onClick={() => onSelect(t.id)}
          >
            {t.label}
            {t.count !== undefined && <span className="count"> {t.count.toLocaleString()}</span>}
          </button>
        ))}
      </div>
      <div role="tabpanel" id={`${idPrefix}-panel`} aria-labelledby={tabId(selected)} tabIndex={0}>
        {children}
      </div>
    </>
  );
}
