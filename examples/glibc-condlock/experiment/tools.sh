# read-only: prints to stdout only, creates no file anywhere, changes nothing, installs nothing
HERMIT=/path/to/hermit   # EDIT: the pinned hermit binary
HERMIT_SHA=666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a
bad=
vge() { [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -n 1)" = "$2" ]; }   # true if version $1 >= $2
chk() { # name need version [opt] : opt = report only, never STOP
  local n=$1 need=$2 v=$3 opt=${4:-} st
  if [ -z "$v" ]; then st=MISSING; elif vge "$v" "$need"; then st=OK; else st=TOO-OLD; fi
  echo "TOOL $n have=${v:-none} need>=$need $st${opt:+ (report only)}"
  [ "$st" = OK ] || [ -n "$opt" ] || bad="$bad $n"; }
need_cmd() { local c; for c in "$@"; do if command -v "$c" > /dev/null 2>&1; then echo "TOOL $c present"; else echo "TOOL $c MISSING"; bad="$bad $c"; fi; done; }
echo "== versions (STOP if missing or too old, except the report-only ones; nothing is installed)"
chk gawk 3.1.2 "$(gawk --version 2> /dev/null | sed -n '1s/^GNU Awk \([0-9][0-9.]*\).*/\1/p')"
# bison and m4 are REPORT ONLY since rev 5: phase T of g0.sh builds a private bison 3.8.2 and m4 1.4.19 from pinned GNU tarballs (no install, no host change); glibc needs bison >= 2.7 and g0.sh verifies the private one
chk bison 2.7 "$(bison --version 2> /dev/null | sed -n '1s/.* \([0-9][0-9.]*\)$/\1/p')" opt
chk m4 1.4.6 "$(m4 --version 2> /dev/null | sed -n '1s/.* \([0-9][0-9.]*\)$/\1/p')" opt
chk make 4.0 "$(make --version 2> /dev/null | sed -n '1s/.* \([0-9][0-9.]*\)$/\1/p')"
chk python3 3.4 "$(python3 -c 'import sys; print("%d.%d.%d" % sys.version_info[:3])' 2> /dev/null)"
chk gcc 6.2 "$(gcc -dumpfullversion 2> /dev/null)"
chk binutils 2.25 "$(ld --version 2> /dev/null | sed -n '1s/.* \([0-9][0-9.]*\)$/\1/p')"
chk perl 5.8 "$(perl -e 'printf "%vd", $^V' 2> /dev/null)"
chk sed 3.02 "$(sed --version 2> /dev/null | sed -n '1s/.* \([0-9][0-9.]*\)$/\1/p')"
chk texinfo 4.7 "$(makeinfo --version 2> /dev/null | sed -n '1s/.* \([0-9][0-9.]*\)$/\1/p')" opt
echo "== exact compiler pin (gcc 13.3.0)"
gv=$(gcc -dumpfullversion 2> /dev/null); echo "TOOL gcc-exact have=${gv:-none} want=13.3.0"; [ "$gv" = 13.3.0 ] || bad="$bad gcc-exact"
echo "== commands"
need_cmd ar nm objdump strip readelf strings as ld ldd sha256sum cmp diff find tar gzip install cp taskset nice timeout setsid pgrep gcc
echo "== kernel headers the build will copy (read only)"
for f in /usr/include/linux/version.h /usr/include/asm-generic/types.h /usr/include/x86_64-linux-gnu/asm/unistd.h; do
  if [ -f "$f" ]; then echo "HDR $f present"; else echo "HDR $f MISSING"; bad="$bad $f"; fi; done
echo "== Hermit pin"
if [ -x "$HERMIT" ]; then hs=$(sha256sum "$HERMIT" | cut -d' ' -f1); echo "HERMIT $HERMIT sha256=$hs want=$HERMIT_SHA"; [ "$hs" = "$HERMIT_SHA" ] || bad="$bad hermit-sha"
else echo "HERMIT $HERMIT MISSING"; bad="$bad hermit"; fi
echo "== space"
fk=$(df -P -k "$HOME" | awk 'NR==2{print $4}'); sk=$(df -P -k /dev/shm | awk 'NR==2{print $4}')
echo "SPACE $HOME avail_kb=$fk need>=31457280; /dev/shm avail_kb=$sk need>=2097152"
[ "${fk:-0}" -ge 31457280 ] || bad="$bad space-home"; [ "${sk:-0}" -ge 2097152 ] || bad="$bad space-shm"
grep ' /dev/shm ' /proc/mounts | sed 's/^/MOUNT /'
ls -d /dev/shm/g0-glibc-* 2> /dev/null | sed 's/^/EXISTING /'
echo "== facts (record only)"
echo "cpu1 governor: $(cat /sys/devices/system/cpu/cpu1/cpufreq/scaling_governor 2>&1) cur_khz: $(cat /sys/devices/system/cpu/cpu1/cpufreq/scaling_cur_freq 2>&1)"
uname -a; head -4 /etc/os-release; ldd --version | head -n 1; nproc; lscpu | grep -E 'Model name|Thread|Core|Socket'
cat /proc/loadavg; cat /proc/mdstat; cat /proc/sys/kernel/perf_event_paranoid
dpkg-query -W libc6 libc6-dev linux-libc-dev gcc-13 binutils gawk bison make python3 texinfo 2>&1
sha256sum /etc/ld.so.cache
ls -la --time-style=full-iso /var/crash
if [ -z "$bad" ]; then echo "TOOLS-VERDICT GO"; else echo "TOOLS-VERDICT STOP:$bad"; fi
