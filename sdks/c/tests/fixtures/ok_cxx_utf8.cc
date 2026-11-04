// UTF-8 in a message is plain text: bytes of 0x80 and above are allowed whatever the signedness of `char` (x86-64
// has it signed, aarch64 unsigned). Built with -c under both -fsigned-char and -funsigned-char (make cxx-utf8), and
// from a directory whose name is not ASCII so that __FILE__ carries such bytes too.
#include "hairspring.h"

void utf8_site() {
  HS_REACHABLE("utf8.reach", "caf\xc3\xa9 \xe2\x82\xac 5 \xe6\x97\xa5\xe6\x9c\xac");
  HS_SOMETIMES(1, "utf8.cond", "na\xc3\xafve");
}
