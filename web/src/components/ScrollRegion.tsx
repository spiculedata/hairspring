import { useLayoutEffect, useRef, useState, type ReactNode, type UIEvent } from 'react';
import { isOverflowing } from './overflow';

interface Props {
  label: string;
  hint: string;
  maxHeight: number;
  onScroll?: (e: UIEvent<HTMLDivElement>) => void;
  children: ReactNode;
}

/**
 * A labelled, keyboard-focusable box that scrolls a wide table, with a hint that is shown exactly while the table is
 * wider than the box. The width is measured, not guessed from a breakpoint: with a page scrollbar the box can be a few
 * pixels narrower than the table at a window width a breakpoint would call wide enough.
 *
 * The box and the table it holds are both observed, and the table is looked up again after every render, so a table
 * that is replaced is observed from then on. Until it has been measured (and where nothing can be observed) the hint is
 * shown: a hint that is not needed costs a line, and a missing one hides that the table scrolls.
 */
export function ScrollRegion({ label, hint, maxHeight, onScroll, children }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const observer = useRef<ResizeObserver | undefined>(undefined);
  const observedTable = useRef<Element | null>(null);
  const [overflowing, setOverflowing] = useState(true);

  // Once, when it mounts: watch the box.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const resize = new ResizeObserver(() => setOverflowing(isOverflowing(el.scrollWidth, el.clientWidth)));
    resize.observe(el);
    observer.current = resize;
    return () => {
      resize.disconnect();
      observer.current = undefined;
      observedTable.current = null;
    };
  }, []);

  // After every render: measure, and watch the table that is there now.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const table = el.firstElementChild;
    if (table !== observedTable.current) {
      if (observedTable.current) observer.current?.unobserve(observedTable.current);
      if (table) observer.current?.observe(table);
      observedTable.current = table;
    }
    setOverflowing(isOverflowing(el.scrollWidth, el.clientWidth));
  });

  return (
    <>
      <p className="scroll-hint" hidden={!overflowing}>
        {hint}
      </p>
      <div
        ref={ref}
        className="table-scroll"
        data-overflowing={overflowing}
        role="region"
        aria-label={label}
        tabIndex={0}
        style={{ maxHeight }}
        onScroll={onScroll}
      >
        {children}
      </div>
    </>
  );
}
