import { plural } from '../wording';

/** Every phrase the Workers page generates. */

export const WORKERS_HEADING = 'Workers';
export const NO_WORKERS = 'No workers registered.';
export const NO_WORKERS_HINT =
  'hairspring serve starts one worker inside the server unless it is given --workers 0. Run hairspring worker to register a remote one.';

export const queueLine = (queued: number, running: number): string =>
  `Queue: ${plural(queued, 'job', 'jobs')} waiting for a worker. ${plural(running, 'job', 'jobs')} held by workers.`;

export const STATE_NOTE =
  'State is worked out when the page loads. Busy: the worker holds a job lease that has not ended. Idle: it holds none and the server heard from it within one lease length (two minutes by default). Not seen recently: it was last heard from longer ago than that.';
export const TRUST_NOTE =
  'Claims need trusted workers. Data from an untrusted worker is shown, labelled as unverified. An administrator sets trust through the API (POST /api/workers/{id}/trust); this page does not change it.';
export const CAPS_NOTE = 'Capabilities are what the worker said about itself when it registered. The server does not check them.';

export const HEADER_NAME = 'Name';
export const HEADER_STATE = 'State';
export const HEADER_TRUST = 'Trust';
export const HEADER_CAPS = 'Capabilities';
export const HEADER_DOING = 'Holding';
export const HEADER_LAST_SEEN = 'Last seen';
export const HEADER_ID = 'Id';

export const HOLDING_NOTHING = 'Nothing';
export const CAPS_NOT_STATED = 'None stated';
export const CAPS_UNREADABLE = 'The worker’s capabilities could not be read as JSON.';
export const CAPS_NOT_REPORTED = 'Not reported by this server';
export const HOLDING_NOT_REPORTED = 'Not reported by this server';
export const STATE_NOT_REPORTED = 'Not reported by this server';
export const TRUST_NOT_REPORTED = 'Not reported by this server';

/** Said to a read caller: the id column is left out, and why. */
export const ID_HIDDEN_NOTE =
  'Worker ids are shown to the run role and above, because an id is a credential for the worker endpoints. Your role is read.';

export const SCROLL_HINT_WORKERS = 'Scroll sideways to see every column.';

export const heldMore = (n: number): string => `and ${n} more`;
export const capabilityMore = (n: number): string => `and ${n} more`;
export const jobKindSuffix = (kind: string): string => (kind === 'run' ? '' : ` (${kind} job)`);
export const attemptsText = (attempts: number): string => (attempts > 1 ? `, attempt ${attempts}` : '');
