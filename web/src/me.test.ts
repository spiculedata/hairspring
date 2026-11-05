import { describe, expect, it } from 'vitest';
import { confirmAccess, isMe, startAccess, type Me, type MeState } from './me';

const me = (role: Me['role'], open = false): Me => ({
  role,
  open_mode: open,
  can: { read: true, run: role !== 'read', admin: role === 'admin' },
  roles: ['read', 'run', 'admin'],
});
const ready = (m: Me): MeState => ({ status: 'ready', me: m });

describe('isMe', () => {
  it('accepts what the server sends for each role and for open mode', () => {
    for (const m of [me('read'), me('run'), me('admin'), me('admin', true)]) expect(isMe(m)).toBe(true);
  });

  it('rejects a body of the wrong shape, so a page never acts on a guess', () => {
    const bad: unknown[] = [
      null,
      5,
      'admin',
      [],
      {},
      { ...me('run'), role: 'root' },
      { ...me('run'), role: undefined },
      { ...me('run'), open_mode: 'no' },
      { ...me('run'), can: null },
      { ...me('run'), can: { read: true, run: 'yes', admin: false } },
      { ...me('run'), can: { read: true, run: true } },
      { ...me('run'), roles: 'read' },
    ];
    for (const b of bad) expect(isMe(b)).toBe(false);
  });
});

describe('startAccess', () => {
  it('lets run and admin callers start, with nothing to explain', () => {
    for (const m of [me('run'), me('admin'), me('admin', true)]) {
      expect(startAccess(ready(m))).toEqual({ canStart: true });
    }
  });

  it('stops a read caller and says, in one line, which role is needed', () => {
    const access = startAccess(ready(me('read')));
    expect(access.canStart).toBe(false);
    expect(access.note).toBe('Your role (read) cannot start runs or sweeps. Ask an administrator for the run role.');
    expect(access.note).not.toContain('\n');
  });

  it('reads the permission from `can`, not from the role name', () => {
    const odd: Me = { ...me('read'), can: { read: true, run: true, admin: false } };
    expect(startAccess(ready(odd)).canStart).toBe(true);
  });

  it('waits while the role is being fetched, and says so', () => {
    expect(startAccess({ status: 'loading' })).toEqual({ canStart: false, note: 'Checking what your role allows…' });
  });

  it('leaves the action to the server when the role cannot be fetched, and does not guess a role', () => {
    expect(startAccess({ status: 'unavailable' })).toEqual({ canStart: true });
  });
});

describe('confirming from the browser', () => {
  const on = (role: Me['role']): Me => ({ ...me(role), can: { ...me(role).can, confirm: true }, confirm: { wall_secs: 120, max_running: 2 } });
  const off = (role: Me['role']): Me => ({ ...me(role), can: { ...me(role).can, confirm: false } });

  it('accepts an answer with can.confirm true or false, and one from a server that has no such key', () => {
    for (const m of [on('run'), off('run'), off('read'), me('run')]) expect(isMe(m)).toBe(true);
  });

  it('rejects can.confirm that is not a boolean, and a true one without its limits', () => {
    const bad: unknown[] = [
      { ...me('run'), can: { ...me('run').can, confirm: 'yes' } },
      { ...me('run'), can: { ...me('run').can, confirm: true } },
      { ...on('run'), confirm: { wall_secs: 'long', max_running: 2 } },
      { ...on('run'), confirm: { wall_secs: 0, max_running: 2 } },
      { ...on('run'), confirm: null },
    ];
    for (const b of bad) expect(isMe(b)).toBe(false);
  });

  it('gives the limits only when the server says this caller may', () => {
    expect(confirmAccess(ready(on('run')))).toEqual({ wall_secs: 120, max_running: 2 });
    expect(confirmAccess(ready(off('run')))).toBeUndefined();
    expect(confirmAccess(ready(me('run')))).toBeUndefined();
    expect(confirmAccess({ status: 'loading' })).toBeUndefined();
    expect(confirmAccess({ status: 'unavailable' })).toBeUndefined();
  });

  it('reads the permission from can.confirm, not from the role name', () => {
    expect(confirmAccess(ready({ ...on('admin'), can: { ...on('admin').can, confirm: false } }))).toBeUndefined();
  });
});
