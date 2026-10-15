#!/usr/bin/env bash
# Size, RAM and startup measurements for the desktop shells. Output: one JSON line per
# measurement on stdout, a human summary on stderr. Work is done by measure-helper.py.
set -euo pipefail

helper="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/measure-helper.py"

usage() {
  cat >&2 <<'EOF'
usage:
  measure.sh size [--label NAME] PATH...
  measure.sh ram [--label NAME] [--env-file FILE] [--pid-pattern REGEX] [--settle S] [--samples N]
                 [--interval S] [--click-window X,Y]... [--screenshot PNG] -- COMMAND...
  measure.sh startup [--label NAME] [--env-file FILE] [--runs N] [--timeout S] [--fresh-dir DIR] -- COMMAND...
EOF
  exit 2
}

[[ $# -ge 1 ]] || usage
case "$1" in
  size|ram|startup) exec python3 "$helper" "$@" ;;
  *) usage ;;
esac
