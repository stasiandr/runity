#!/bin/sh
# Opens the editor at the checkout's current state: builds when the sources
# changed since the last build, then opens target/release/Scrap.app. Cargo
# decides what is stale, so an unchanged checkout opens in a second or so.
#
# The launcher from `tools/studio-app.sh --launcher` (an AppleScript applet,
# tools/studio-launcher.applescript) drives this in steps, so that it can show
# the build's progress in a window:
#
#   studio-open.sh start   # "running" when the editor is open, else starts the build
#   studio-open.sh poll    # "building|built|to build|what|elapsed", or "done|exit code"
#   studio-open.sh stop    # the build, cancelled
#   studio-open.sh open    # the editor
#   studio-open.sh log     # the build's log, in Console
#
# With no step it does all of them in a row, without a window.
set -u
cd "$(dirname "$0")/.."
# From Finder or hop an app gets a bare /usr/bin:/bin.
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"

app=target/release/Scrap.app
state="$HOME/Library/Logs/scrap-studio"
log="$state/build.log"
mkdir -p "$state"

running() { pgrep -f "$app/Contents/MacOS/scrap-studio" >/dev/null; }

# The build and everything under it.
kill_tree() {
  for child in $(pgrep -P "$1"); do kill_tree "$child"; done
  kill "$1" 2>/dev/null
}

case "${1:-}" in
  start)
    # Already open: the running editor comes to the front; a rebuild would
    # not reach it anyway.
    if running; then echo running; exit 0; fi
    [ -f "$state/pid" ] && kill_tree "$(cat "$state/pid")"
    rm -f "$state/status" "$state/base"
    date +%s > "$state/started"
    # Cargo draws its bar only on a terminal unless told to.
    ( CARGO_TERM_PROGRESS_WHEN=always CARGO_TERM_PROGRESS_WIDTH=200 tools/studio-app.sh >"$log" 2>&1
      echo $? > "$state/status" ) </dev/null >/dev/null 2>&1 &
    echo $! > "$state/pid"
    echo started
    ;;
  poll)
    if [ -f "$state/status" ]; then
      echo "done|$(cat "$state/status")"
      exit 0
    fi
    # Nothing compiled yet: cargo is still checking what is stale.
    grep -q 'Compiling' "$log" 2>/dev/null || { echo checking; exit 0; }
    elapsed=$(( $(date +%s) - $(cat "$state/started") ))
    # "    Building [=====>   ] 432/435: scrap-engine, scrap-ui"
    last=$(tr '\r' '\n' < "$log" | grep 'Building \[' | tail -1 \
      | sed -E 's/.*\] ([0-9]+)\/([0-9]+): *(.*[^ ]) *$/\1|\2|\3/')
    clock=$(printf '%d:%02d' $((elapsed / 60)) $((elapsed % 60)))
    case "$last" in
      *'|'*'|'*) ;;
      *) echo "building|0|0||$clock"; exit 0 ;;
    esac
    done_units=${last%%|*}; rest=${last#*|}; all=${rest%%|*}; what=${rest#*|}
    # Cargo counts the fresh crates too; the bar is for the ones this build
    # compiles, counted from where cargo was when first seen.
    [ -f "$state/base" ] || echo $((done_units > 0 ? done_units - 1 : 0)) > "$state/base"
    base=$(cat "$state/base")
    echo "building|$((done_units - base))|$((all - base))|$what|$clock"
    ;;
  stop)
    [ -f "$state/pid" ] && kill_tree "$(cat "$state/pid")"
    rm -f "$state/pid"
    ;;
  open)
    open "$app"
    ;;
  log)
    open -a Console "$log"
    ;;
  "")
    running || tools/studio-app.sh >"$log" 2>&1 || { echo "build failed, see $log" >&2; exit 1; }
    open "$app"
    ;;
  *)
    echo "usage: $0 [start|poll|stop|open|log]" >&2
    exit 2
    ;;
esac
