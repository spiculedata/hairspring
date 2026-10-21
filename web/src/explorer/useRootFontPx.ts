import { useLayoutEffect, useState } from 'react';
import { DEFAULT_ROOT_FONT_PX, parseFontPx } from './rowHeight';

/**
 * The browser's root font size in pixels, which a reader's default font size and zoom change and which sizes the `rem`
 * text that decides how tall a property row has to be (rowHeight.ts). The default is used until it has been measured.
 */
export function useRootFontPx(): number {
  const [px, setPx] = useState(DEFAULT_ROOT_FONT_PX);
  useLayoutEffect(() => {
    const read = () => setPx(parseFontPx(getComputedStyle(document.documentElement).fontSize));
    read();
    window.addEventListener('resize', read);
    return () => window.removeEventListener('resize', read);
  }, []);
  return px;
}
