#!/usr/bin/env python3
"""Insert sched_yield() scheduling points into SQLite's WAL restart/checkpoint code (walwork-yield build)."""
import sys
src, dst = sys.argv[1], sys.argv[2]
s = open(src).read()
def sub(old, new):
    global s
    assert s.count(old) == 1, (s.count(old), old[:60])
    s = s.replace(old, new)
sub('#define SQLITE_CORE 1', '#define SQLITE_CORE 1\n#include <sched.h>\n#define HS_YIELD() sched_yield()') if '#define SQLITE_CORE 1' in s and s.count('#define SQLITE_CORE 1') == 1 else None
if 'HS_YIELD()' not in s.split('\n', 40)[0:40].__str__():
    s = 'extern int sched_yield(void);\n#define HS_YIELD() sched_yield()\n' + s
sub('  pWal->hdr.mxFrame = 0;\n  sqlite3Put4byte((u8*)&aSalt[0]', '  HS_YIELD();\n  pWal->hdr.mxFrame = 0;\n  sqlite3Put4byte((u8*)&aSalt[0]')
sub('  walIndexWriteHdr(pWal);\n  AtomicStore(&pInfo->nBackfill, 0);\n  pInfo->nBackfillAttempted = 0;\n  pInfo->aReadMark[1] = 0;',
    '  walIndexWriteHdr(pWal);\n  HS_YIELD();\n  AtomicStore(&pInfo->nBackfill, 0);\n  HS_YIELD();\n  pInfo->nBackfillAttempted = 0;\n  HS_YIELD();\n  pInfo->aReadMark[1] = 0;')
sub('  pInfo = walCkptInfo(pWal);\n  if( pInfo->nBackfill<pWal->hdr.mxFrame ){', '  pInfo = walCkptInfo(pWal);\n  HS_YIELD();\n  if( pInfo->nBackfill<pWal->hdr.mxFrame ){')
sub('          AtomicStore(&pInfo->nBackfill, mxSafeFrame); SEH_INJECT_FAULT;', '          HS_YIELD();\n          AtomicStore(&pInfo->nBackfill, mxSafeFrame); SEH_INJECT_FAULT;')
sub('    SEH_INJECT_FAULT;\n    if( pInfo->nBackfill<pWal->hdr.mxFrame ){\n      rc = SQLITE_BUSY;', '    SEH_INJECT_FAULT;\n    HS_YIELD();\n    if( pInfo->nBackfill<pWal->hdr.mxFrame ){\n      rc = SQLITE_BUSY;')
sub('        walRestartHdr(pWal, salt1);\n        walUnlockExclusive(pWal, WAL_READ_LOCK(1), WAL_NREADER-1);\n      }else if', '        HS_YIELD();\n        walRestartHdr(pWal, salt1);\n        walUnlockExclusive(pWal, WAL_READ_LOCK(1), WAL_NREADER-1);\n      }else if')
open(dst, 'w').write(s)
print('patched', dst, s.count('HS_YIELD()'))
