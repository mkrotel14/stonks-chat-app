#!/usr/bin/env bash
set -euo pipefail

launcher=${1:?Pass the executable or AppImage}
required_visible_checks=1
if [[ ${2:-} == --require-stable-window ]]; then
  # Mapping the window can precede a crash in first-frame shader compilation.
  required_visible_checks=4
fi
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
visible_checks=0
for ((i = 0; i < 60; i++)); do
  if ! kill -0 "$pid" 2>/dev/null; then break; fi
  if xdotool search --onlyvisible --name '^Stonks Chat$' >/dev/null 2>&1; then
    if (( ++visible_checks >= required_visible_checks )); then
      echo "Visible Stonks Chat window: $launcher"
      exit 0
    fi
  else
    visible_checks=0
  fi
  sleep 1
done
cat "$state/startup.log" >&2
echo "No visible Stonks Chat window met the smoke check: $launcher" >&2
exit 1
