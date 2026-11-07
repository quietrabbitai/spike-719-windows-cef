#!/bin/bash
# items.id=720 (THROWAWAY): per-process sandbox report for a running spike720 tree.
#   tools/check-sandbox.sh <main-pid>      (run while the app is up)
# Controls: pid 1 (launchd) must read 0; the WKWebView's own WebKit.WebContent process (a platform-sandboxed
# process spawned by this very app) must read 1. If a control is wrong the result below is not trustworthy.
cd "$(dirname "$0")/.."
MAIN="$1"
[ -n "$MAIN" ] || { echo "usage: $0 <main-pid>"; exit 1; }
echo "== controls"
printf 'launchd (negative control): '; tools/out/sandbox_check 1
for p in $(pgrep -f 'com.apple.WebKit.WebContent' | head -3); do printf 'WebKit.WebContent (positive control) '; tools/out/sandbox_check "$p"; done
echo "== spike720 tree (main pid $MAIN)"
printf '%-7s %-9s %s\n' PID SANDBOXED ROLE
walk() { for c in $(pgrep -P "$1"); do echo "$c"; walk "$c"; done; }
for p in "$MAIN" $(walk "$MAIN"); do
  cmd=$(ps -o command= -p "$p" 2>/dev/null)
  role=$(echo "$cmd" | grep -o -- '--type=[a-z-]*' | head -1); [ -n "$role" ] || role="(browser/main)"
  extra=$(echo "$cmd" | grep -o -- '--utility-sub-type=[A-Za-z.]*' | head -1)
  s=$(tools/out/sandbox_check "$p" | sed 's/.*sandboxed=//')
  printf '%-7s %-9s %s %s\n' "$p" "$s" "$role" "$extra"
done
