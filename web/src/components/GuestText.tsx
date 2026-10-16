import { useState } from 'react';
import { escapeGuest, GUEST_MAX_EXPANDED, GUEST_MAX_INLINE } from '../guest/escape';
import { GUEST_SOURCE_NOTE, SHOW_ALL, SHOW_LESS, truncationMarker } from '../wording';

interface Props {
  /** Text from the program under test. Untrusted. */
  text: string;
  /** Characters shown before the rest is cut. */
  max?: number;
  /** Offer "show all" (up to 64 KiB). Off where a row has a fixed height. */
  expandable?: boolean;
}

/**
 * The only place the explorer renders strings that come from the program under test.
 *
 * It renders React text nodes and nothing else: no innerHTML, no links, no attributes built from the text, no
 * markdown. Control, ANSI, bidi and format characters are shown as visible escapes, and long text is cut with a marker
 * that says how much was left out. Anything that shows guest text must come through here.
 */
export function GuestText({ text, max = GUEST_MAX_INLINE, expandable = true }: Props) {
  const [expanded, setExpanded] = useState(false);
  const limit = expanded ? GUEST_MAX_EXPANDED : max;
  const { segments, truncated, omitted } = escapeGuest(text, limit);
  const canExpand = expandable && !expanded && truncated && max < GUEST_MAX_EXPANDED;
  const canCollapse = expandable && expanded && text.length > max;
  return (
    <span className="guest-text" title={`${GUEST_SOURCE_NOTE} (untrusted text)`}>
      {segments.map((s, i) =>
        s.escaped ? (
          <span key={i} className="guest-esc">
            {s.text}
          </span>
        ) : (
          s.text
        ),
      )}
      {truncated && <span className="guest-trunc">{truncationMarker(omitted)}</span>}
      {canExpand && (
        <>
          {' '}
          <button type="button" className="linklike guest-more" onClick={() => setExpanded(true)}>
            {SHOW_ALL}
          </button>
        </>
      )}
      {canCollapse && (
        <>
          {' '}
          <button type="button" className="linklike guest-more" onClick={() => setExpanded(false)}>
            {SHOW_LESS}
          </button>
        </>
      )}
    </span>
  );
}
