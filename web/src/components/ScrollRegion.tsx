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
 * Until it has been measured (and where nothing can be measured) the hint is shown: a hint that is not needed costs a
 * line, and a missing one hides that the table scrolls.
 */
export function ScrollRegion({ label, hint, maxHeight, onScroll, children }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [overflowing, setOverflowing] = useState(true);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setOverflowing(isOverflowing(el.scrollWidth, el.clientWidth));
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    const table = el.firstElementChild;
    if (table) observer.observe(table);
    return () => observer.disconnect();
  }, []);

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
