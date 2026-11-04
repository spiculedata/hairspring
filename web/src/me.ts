/** Who the server says the caller is (`GET /api/me`). The role is never worked out from a failed request. */
export type RoleName = 'read' | 'run' | 'admin';

export interface Me {
  readonly role: RoleName;
  /** The server has no API tokens and accepts only same-origin local requests; every caller is then an admin. */
  readonly open_mode: boolean;
  readonly can: { readonly read: boolean; readonly run: boolean; readonly admin: boolean };
  readonly roles: readonly string[];
}

/** What the page knows about the caller. `unavailable` means the question could not be answered (an older server, an error). */
export type MeState = { readonly status: 'loading' } | { readonly status: 'ready'; readonly me: Me } | { readonly status: 'unavailable' };

const ROLE_NAMES: readonly string[] = ['read', 'run', 'admin'];

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/** Checks the parts of a `/api/me` response the UI reads, so a wrong-shaped body is an error and not a guess. */
export function isMe(v: unknown): v is Me {
  if (!isObject(v) || typeof v.role !== 'string' || !ROLE_NAMES.includes(v.role)) return false;
  if (typeof v.open_mode !== 'boolean' || !Array.isArray(v.roles) || !isObject(v.can)) return false;
  return typeof v.can.read === 'boolean' && typeof v.can.run === 'boolean' && typeof v.can.admin === 'boolean';
}

/** Whether an action is offered, and the one line that says why not. */
export interface Access {
  readonly canStart: boolean;
  readonly note?: string;
}

export const CHECKING_ROLE = 'Checking what your role allows…';

export function cannotStartNote(role: RoleName): string {
  return `Your role (${role}) cannot start runs or sweeps. Ask an administrator for the run role.`;
}

/**
 * Starting a run or a sweep needs the run role. While the role is being fetched the button waits. If the role cannot
 * be fetched nothing is assumed: the button stays available and the server still decides.
 */
export function startAccess(state: MeState): Access {
  if (state.status === 'loading') return { canStart: false, note: CHECKING_ROLE };
  if (state.status === 'unavailable' || state.me.can.run) return { canStart: true };
  return { canStart: false, note: cannotStartNote(state.me.role) };
}
