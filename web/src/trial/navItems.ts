export interface NavItem {
  /** The id of the heading the item leads to. */
  readonly id: string;
  readonly label: string;
}

const NEEDS_DETAIL: readonly NavItem[] = [
  { id: 'witness-title', label: 'Witness' },
  { id: 'threads-title', label: 'Threads' },
  { id: 'procs-title', label: 'Processes' },
  { id: 'health-title', label: 'Event health' },
];

const ALWAYS: readonly NavItem[] = [
  { id: 'schedule-title', label: 'Schedule' },
  { id: 'output-title', label: 'Output' },
  { id: 'about-title', label: 'About this page' },
];

/** The sections of the page in the order they appear; the four that need kept detail are left out when none was kept. */
export function navItems(hasDetail: boolean): readonly NavItem[] {
  return hasDetail ? [...NEEDS_DETAIL, ...ALWAYS] : ALWAYS;
}
