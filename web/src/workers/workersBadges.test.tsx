import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { StatusBadge } from '../components/StatusBadge';
import { workerStateEntry, workerTrustEntry } from '../statusMap';

describe('worker badges', () => {
  it.each([
    ['busy', 'Busy', 'busy'],
    ['idle', 'Idle', 'idle'],
    ['not_seen_recently', 'Not seen recently', 'gap'],
  ])('labels the server word %s as text with its own tone', (word, label, tone) => {
    expect(workerStateEntry(word)).toEqual({ label, tone });
    const html = renderToString(<StatusBadge kind="workerState" state={word} />);
    expect(html).toContain(`badge-${tone}`);
    expect(html).toContain(`>${label}<`);
    expect(html).toContain(`Server state: ${word}`);
  });

  it('shows a word it does not know as a neutral Unknown, and never an inherited property', () => {
    expect(workerStateEntry('hibernating')).toEqual({ label: 'Unknown', tone: 'idle' });
    expect(workerStateEntry('constructor')).toEqual({ label: 'Unknown', tone: 'idle' });
    expect(workerStateEntry('__proto__')).toEqual({ label: 'Unknown', tone: 'idle' });
  });

  it('gives trust a word as well as a tone, and marks untrusted as caution', () => {
    expect(workerTrustEntry(true)).toEqual({ label: 'Trusted', tone: 'good' });
    expect(workerTrustEntry(false)).toEqual({ label: 'Untrusted', tone: 'gap' });
    expect(renderToString(<StatusBadge kind="workerTrust" trusted={false} />)).toContain('>Untrusted<');
  });
});
