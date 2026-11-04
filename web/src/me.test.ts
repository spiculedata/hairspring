import { describe, expect, it } from 'vitest';
import { isMe, startAccess, type Me, type MeState } from './me';

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
