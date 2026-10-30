import { useEffect } from 'react';

/** Sets the document title while the calling page is shown. */
export function useDocumentTitle(title: string): void {
  useEffect(() => {
    document.title = title;
  }, [title]);
}
