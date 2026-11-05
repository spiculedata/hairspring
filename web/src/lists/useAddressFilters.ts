import { useCallback, useEffect, useRef, useState } from 'react';
import { onAddressChange, replaceAddress } from '../router';

/**
 * The filters of a list page, kept in step with the address: they start from the address, they are written back to it as
 * they change (in place, adding no history entry), and a link followed in the page or Back and Forward that changes
 * the address to this page's own with other filters sets them. Anything invalid in the address is left out.
 *
 * An address that belongs to another page is never touched: the page this hook is on is the one it was mounted on.
 */
export function useAddressFilters<F>(parse: (search: string) => F, toSearch: (filters: F) => string): [F, (next: F) => void] {
  const path = useRef(window.location.pathname);
  const [filters, setFilters] = useState<F>(() => parse(window.location.search));
  const search = toSearch(filters);

  useEffect(() => {
    if (window.location.pathname === path.current) replaceAddress(`${path.current}${search}`);
  }, [search]);

  useEffect(
    () =>
      onAddressChange(() => {
        if (window.location.pathname !== path.current) return;
        const next = parse(window.location.search);
        setFilters((held) => (toSearch(held) === toSearch(next) ? held : next));
      }),
    // `parse` and `toSearch` are the page's own functions and do not change.
    [],
  );

  const change = useCallback((next: F) => setFilters(next), []);
  return [filters, change];
}
