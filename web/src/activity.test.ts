import { describe, expect, it } from 'vitest';
import { describeEvent } from './activity';

const ev = (event: string, data: string) => ({ id: '1', event, data });

describe('describeEvent', () => {
  it('words a started event with its worker', () => {
    expect(describeEvent(ev('started', '{"worker":"local-55774"}'))).toEqual({ title: 'Started', facts: [['Worker', 'local-55774']] });
  });

  it('words a progress event with its message and counts', () => {
    expect(describeEvent(ev('progress', '{"message":"1 of 40 trials run","steps":1200,"trials":1}'))).toEqual({
      title: 'Progress',
      facts: [['Message', '1 of 40 trials run'], ['Steps', '1,200'], ['Trials', '1']],
    });
  });

  it('words a finished event in plain labels', () => {
    const d = describeEvent(ev('finished', '{"distinct_schedules":3,"inconclusive":0,"state":"failed","work":0}'));
    expect(d.title).toBe('Finished');
    expect(d.facts).toContainEqual(['State', 'failed']);
    expect(d.facts).toContainEqual(['Distinct schedules', '3']);
    expect(d.facts).toContainEqual(['Inconclusive trials', '0']);
  });

  it('shows a key it does not know with the underscores read as spaces, and never drops it', () => {
    expect(describeEvent(ev('finished', '{"some_new_key":true}')).facts).toContainEqual(['some new key', 'yes']);
    expect(describeEvent(ev('finished', '{"x":{"a":1}}')).facts).toContainEqual(['x', '{"a":1}']);
  });

  it('keeps an event it does not know, titled by its name', () => {
    expect(describeEvent(ev('retry', '{"attempt":2}'))).toEqual({ title: 'Retry', facts: [['attempt', '2']] });
  });

  it('shows data that is not a JSON object as it came, so nothing is lost', () => {
    expect(describeEvent(ev('progress', 'plain text'))).toEqual({ title: 'Progress', facts: [['Data', 'plain text']] });
    expect(describeEvent(ev('progress', '[1,2]'))).toEqual({ title: 'Progress', facts: [['Data', '[1,2]']] });
    expect(describeEvent(ev('message', ''))).toEqual({ title: 'Message', facts: [] });
  });

  it('cuts a very long value', () => {
    const d = describeEvent(ev('progress', JSON.stringify({ message: 'x'.repeat(5000) })));
    expect(d.facts[0][1].length).toBeLessThan(600);
  });
});
