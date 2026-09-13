#!/usr/bin/env bash
# Run a command and, if it fails, put the tail of its output in a GitHub
# annotation.
#
# A failed job otherwise says "Process completed with exit code 101" to
# anybody who cannot download the log, and downloading the log needs a token.
# Annotations are readable from the API without one, so the reason travels
# with the failure instead of staying behind a login.
set -uo pipefail

log=$(mktemp)
"$@" 2>&1 | tee "$log"
code=${PIPESTATUS[0]}

if [ "$code" -ne 0 ]; then
  python3 - "$log" <<'PY'
import sys

KEEP = 3500  # annotations are capped; the end is the part that says why
text = open(sys.argv[1], encoding="utf-8", errors="replace").read()[-KEEP:]
escaped = text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
print(f"::error title=ci::{escaped}")
PY
fi
exit "$code"
