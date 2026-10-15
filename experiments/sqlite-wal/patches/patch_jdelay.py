#!/usr/bin/env python3
"""Calibration build v2: deterministic jittered delay inside walCheckpoint's race window, taken only on a
'dangerous' checkpoint (nBackfill == mxFrame, i.e. fully backfilled WAL). Sleeps base + (n*7919 % spread) microseconds,
n = per-call counter, so wake phases vary across cycles without any randomness. Env: HS_DELAY_BASE_US, HS_DELAY_SPREAD_US."""
import sys
src, dst = sys.argv[1], sys.argv[2]
s = open(src).read()
old = '  pInfo = walCkptInfo(pWal);\n  if( pInfo->nBackfill<pWal->hdr.mxFrame ){'
assert s.count(old) == 1
s = s.replace(old, '  pInfo = walCkptInfo(pWal);\n  if( pInfo->nBackfill==pWal->hdr.mxFrame && pWal->hdr.mxFrame>0 ) hs_delay();\n  if( pInfo->nBackfill<pWal->hdr.mxFrame ){')
prelude = ('extern int usleep(unsigned int);\nextern char *getenv(const char *);\nextern int atoi(const char *);\n'
           'static void hs_delay(void){ static int base=-1, spread=1; static unsigned long n=0; if(base<0){ const char *b=getenv("HS_DELAY_BASE_US"), *p=getenv("HS_DELAY_SPREAD_US"); base=b?atoi(b):0; spread=p?atoi(p):1; if(spread<1) spread=1; }'
           ' if(base>0){ unsigned long d = base + (unsigned long)((n++*7919UL) % (unsigned long)spread); usleep((unsigned int)d); } }\n')
open(dst, 'w').write(prelude + s)
print('patched', dst)
