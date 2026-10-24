#!/bin/sh
# Checks that the text of `java -version` (on stdin) names exactly the pinned JDK build.
#
#   java -version 2>&1 | check-jdk-version.sh <pinned-build>      e.g. Temurin-21.0.12.1+1
#
# The pinned string must be followed by a space, a closing bracket or the end of the line, so Temurin-21.0.12.1+1 does not
# accept Temurin-21.0.12.1+10 or +19; its dots and plus sign are matched literally.
# Exits 0 when the build is the pinned one, 1 when not, 64 for a usage error.

set -u

if [ $# -ne 1 ] || [ -z "$1" ]; then
  echo "usage: $0 <pinned-build> < java-version-text" >&2
  exit 64
fi
escaped=$(printf '%s' "$1" | sed 's#[][\.*^$+?(){}|/]#\\&#g')
if grep -Eq "${escaped}( |\)|\$)"; then
  exit 0
fi
echo "FAIL the JDK is not the pinned $1" >&2
exit 1
