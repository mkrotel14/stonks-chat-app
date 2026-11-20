#!/usr/bin/env bash
set -euo pipefail

launcher=${1:?Pass the executable or AppImage}
state=$(mktemp -d)
export HOME=$state XDG_CONFIG_HOME=$state/config XDG_DATA_HOME=$state/data
export XDG_CACHE_HOME=$state/cache XDG_RUNTIME_DIR=$state/run
export STONKS_ALLOW_MULTI_INSTANCE=1 APPIMAGE_EXTRACT_AND_RUN=1
unset WAYLAND_DISPLAY WAYLAND_SOCKET
mkdir -m 700 "$XDG_RUNTIME_DIR"
setsid "$launcher" --automatic-renderer >"$state/startup.log" 2>&1 &
pid=$!
cleanup() {
  kill -- "-$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  rm -rf "$state"
}
trap cleanup EXIT
for ((i = 0; i < 60; i++)); do
  if ! kill -0 "$pid" 2>/dev/null; then break; fi
  if xdotool search --onlyvisible --name '^Stonks Chat$' >/dev/null 2>&1; then
    echo "Visible Stonks Chat window: $launcher"
    exit 0
  fi
  sleep 1
done
cat "$state/startup.log" >&2
echo "No visible Stonks Chat window: $launcher" >&2
exit 1
