import { describe, expect, it } from 'vitest';
import { parseSse } from './sse';

describe('parseSse', () => {
  it('parses complete events and keeps the unfinished remainder', () => {
    const { events, rest } = parseSse('id: 1\nevent: started\ndata: {"a":1}\n\nid: 2\nevent: progre');
    expect(events).toEqual([{ id: '1', event: 'started', data: '{"a":1}' }]);
    expect(rest).toBe('id: 2\nevent: progre');
  });
  it('handles CRLF, multi-line data and comments', () => {
    const { events } = parseSse(': keep-alive\r\n\r\nevent: x\r\ndata: a\r\ndata: b\r\n\r\n');
    expect(events).toEqual([{ id: '', event: 'x', data: 'a\nb' }]);
  });
  it('defaults the event name to message and ignores empty blocks', () => {
    const { events, rest } = parseSse('data: hi\n\n\n\n');
    expect(events).toEqual([{ id: '', event: 'message', data: 'hi' }]);
    expect(rest).toBe('');
  });
  it('is stable when fed incrementally', () => {
    const a = parseSse('event: a\ndata: 1\n');
    expect(a.events).toEqual([]);
    const b = parseSse(a.rest + '\nevent: b\ndata: 2\n\n');
    expect(b.events.map((e) => e.event)).toEqual(['a', 'b']);
  });
});
