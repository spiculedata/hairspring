#!/usr/bin/env python3
"""Validation build: widen the WAL-reset race window by sleeping HS_DELAY_US microseconds in walCheckpoint,
between the cached-header read and the nBackfill check. Used to prove the workload can see the bug."""
import sys
src, dst = sys.argv[1], sys.argv[2]
s = open(src).read()
old = '  pInfo = walCkptInfo(pWal);\n  if( pInfo->nBackfill<pWal->hdr.mxFrame ){'
assert s.count(old) == 1
s = s.replace(old, '  pInfo = walCkptInfo(pWal);\n  hs_delay();\n  if( pInfo->nBackfill<pWal->hdr.mxFrame ){')
prelude = ('extern int usleep(unsigned int);\nextern char *getenv(const char *);\nextern int atoi(const char *);\n'
           'static void hs_delay(void){ static int us = -1; if(us<0){ const char *e = getenv("HS_DELAY_US"); us = e ? atoi(e) : 0; } if(us>0) usleep(us); }\n')
marker = '#ifndef SQLITE_AMALGAMATION'
i = s.find('/************** Begin file sqliteInt.h')
assert i > 0
s = prelude + s
open(dst, 'w').write(s)
print('patched', dst)
