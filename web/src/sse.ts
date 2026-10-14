export interface SseEvent {
  id: string;
  event: string;
  data: string;
}

/** Splits a buffer into complete server-sent events plus the unfinished remainder. */
export function parseSse(buffer: string): { events: SseEvent[]; rest: string } {
  const normalized = buffer.replace(/\r\n/g, '\n');
  const blocks = normalized.split('\n\n');
  const rest = blocks.pop() ?? '';
  const events: SseEvent[] = [];
  for (const block of blocks) {
    let id = '';
    let event = 'message';
    const data: string[] = [];
    for (const line of block.split('\n')) {
      if (line === '' || line.startsWith(':')) continue;
      const colon = line.indexOf(':');
      const field = colon === -1 ? line : line.slice(0, colon);
      const value = colon === -1 ? '' : line.slice(colon + 1).replace(/^ /, '');
      if (field === 'id') id = value;
      else if (field === 'event') event = value;
      else if (field === 'data') data.push(value);
    }
    if (data.length > 0 || id !== '') events.push({ id, event, data: data.join('\n') });
  }
  return { events, rest };
}
