/**
 * Named characters for tests, built from code points so no invisible or bidi character sits in a source file.
 * Only tests import this; it carries no behaviour.
 */
const c = (code: number): string => String.fromCodePoint(code);

export const CH = {
  NUL: c(0x0),
  BEL: c(0x7),
  TAB: c(0x9),
  LF: c(0xa),
  ESC: c(0x1b),
  DEL: c(0x7f),
  NEL: c(0x85),
  ZWSP: c(0x200b),
  ZWJ: c(0x200d),
  LRM: c(0x200e),
  LS: c(0x2028),
  PS: c(0x2029),
  RLO: c(0x202e),
  LRI: c(0x2066),
  PDI: c(0x2069),
  BOM: c(0xfeff),
  LONE_SURROGATE: String.fromCharCode(0xd800),
  GRIN: c(0x1f600),
} as const;
