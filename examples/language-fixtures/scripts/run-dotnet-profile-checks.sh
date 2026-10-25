#!/bin/sh
# The native self-check of the .NET fixture (run-checks.sh: the contract, 50 runs, a negative control) once for each
# environment profile arm of profiles.manifest (arm a, then arm b), with that arm's variables added to the environment.
#
#   run-dotnet-profile-checks.sh <profiles.manifest> <LangAt.dll> <dotnet> [runs]
#
# The program is run as `<dotnet> LangAt.dll <mode>` through a small wrapper, because run-checks.sh runs one executable that
# takes the mode as its argument. profile-env.sh prints the arm's variables for the runtime directory of <dotnet> (the
# directory that holds the dotnet executable, links resolved). RUN_CHECKS names the checker (default: run-checks.sh beside
# this script); the script tests use it. Both arms are run even after one fails. Exits 0 when both passed, 1 when one failed
# or the manifest is bad, 64 for a usage error.

set -u

if [ $# -lt 3 ] || [ $# -gt 4 ]; then
  echo "usage: $0 <profiles.manifest> <LangAt.dll> <dotnet> [runs]" >&2
  exit 64
fi
here=$(cd "$(dirname "$0")" && pwd)
manifest=$1
dll=$2
dotnet=$3
runs=${4:-50}
checker=${RUN_CHECKS:-$here/run-checks.sh}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/dotnet-profiles.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
root=$(dirname "$(readlink -f "$dotnet" 2>/dev/null || echo "$dotnet")")
status=0

for arm in a b; do
  if ! words=$(sh "$here/../dotnet/profile-env.sh" "$arm" "$root" "$manifest"); then
    echo "FAIL arm $arm: the manifest $manifest is bad"
    status=1
    continue
  fi
  wrapper=$tmp/at-dotnet-$arm
  {
    printf '#!/bin/sh\nexec env'
    printf '%s\n' "$words" | while IFS= read -r word; do
      quoted=$(printf '%s' "$word" | sed "s/'/'\\\\''/g")
      printf " '%s'" "$quoted"
    done
    quoted_dotnet=$(printf '%s' "$dotnet" | sed "s/'/'\\\\''/g")
    quoted_dll=$(printf '%s' "$dll" | sed "s/'/'\\\\''/g")
    printf " '%s' '%s' \"\$@\"\n" "$quoted_dotnet" "$quoted_dll"
  } >"$wrapper"
  chmod +x "$wrapper"
  echo "--- arm $arm: $(printf '%s' "$words" | tr '\n' ' ')"
  if sh "$checker" dotnet "$wrapper" "$runs"; then
    echo "ok   arm $arm: the self-check passed under these variables"
  else
    echo "FAIL arm $arm: the self-check failed under these variables"
    status=1
  fi
done
exit $status
