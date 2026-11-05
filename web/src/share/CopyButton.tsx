import { useState } from 'react';

type CopyState = 'idle' | 'copied' | 'failed';

const COPIED = 'Copied';
const FAILED = 'Could not copy. Select the text on the page and copy it yourself.';

export interface CopyItem {
  label: string;
  /** Read when the button is pressed, so the text is never stale. */
  text: () => string;
}

async function writeToClipboard(read: () => string): Promise<CopyState> {
  try {
    // Reading the text is inside the try: a text() that throws is a failed copy, and says so.
    const text = read();
    if (navigator.clipboard === undefined) throw new Error('no clipboard');
    await navigator.clipboard.writeText(text);
    return 'copied';
  } catch {
    return 'failed';
  }
}

/**
 * Copy buttons that share one live region, so pressing a second one replaces the message of the first instead of adding
 * another. A refused or missing clipboard is never shown as success.
 */
export function CopyButtons({ items }: { items: readonly CopyItem[] }) {
  const [state, setState] = useState<CopyState>('idle');
  return (
    <>
      {items.map((item) => (
        <button key={item.label} type="button" onClick={() => void writeToClipboard(item.text).then(setState)}>
          {item.label}
        </button>
      ))}
      <span role="status" className="muted copy-status">
        {state === 'copied' ? COPIED : state === 'failed' ? FAILED : ''}
      </span>
    </>
  );
}

/** One copy button with its own message. Use `CopyButtons` where several sit together. */
export function CopyButton({ label, text }: CopyItem) {
  return <CopyButtons items={[{ label, text }]} />;
}
