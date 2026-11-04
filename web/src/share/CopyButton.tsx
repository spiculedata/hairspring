import { useState } from 'react';

type CopyState = 'idle' | 'copied' | 'failed';

const COPIED = 'Copied';
const FAILED = 'Could not copy. Select the text on the page and copy it yourself.';

interface Props {
  label: string;
  /** Read when the button is pressed, so the text is never stale. */
  text: () => string;
}

/** Copies text to the clipboard and says whether that worked: a refused or missing clipboard is never shown as success. */
export function CopyButton({ label, text }: Props) {
  const [state, setState] = useState<CopyState>('idle');
  const copy = async () => {
    try {
      if (navigator.clipboard === undefined) throw new Error('no clipboard');
      await navigator.clipboard.writeText(text());
      setState('copied');
    } catch {
      setState('failed');
    }
  };
  return (
    <>
      <button type="button" onClick={() => void copy()}>
        {label}
      </button>
      <span role="status" className="muted copy-status">
        {state === 'copied' ? COPIED : state === 'failed' ? FAILED : ''}
      </span>
    </>
  );
}
