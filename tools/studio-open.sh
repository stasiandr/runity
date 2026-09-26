#!/bin/sh
# Opens the editor at the checkout's current state: builds when the sources
# changed since the last build, then opens target/release/Scrap.app. Cargo
# decides what is stale, so an unchanged checkout opens in a second or so.
# The launcher from `tools/studio-app.sh --launcher` runs this.
set -u
cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"

app=target/release/Scrap.app
log="$HOME/Library/Logs/scrap-studio-build.log"
notify() {
  terminal-notifier -title Scrap -message "$1" -group scrap-build >/dev/null 2>&1 \
    || osascript -e "display notification \"$1\" with title \"Scrap\"" >/dev/null 2>&1
}

# Already open: the running editor comes to the front; a rebuild would not
# reach it anyway.
if pgrep -f "$app/Contents/MacOS/scrap-studio" >/dev/null; then
  exec open "$app"
fi

tools/studio-app.sh >"$log" 2>&1 &
build=$!
# Only a real build is announced, not cargo's second of checking.
( sleep 3; kill -0 $build 2>/dev/null && notify "Собираю редактор…" ) &
if wait $build; then
  exec open "$app" --args "$@"
fi

if [ -d "$app" ]; then
  answer=$(osascript -e 'button returned of (display dialog "Сборка Scrap упала." buttons {"Лог", "Открыть прежнюю"} default button 2 with title "Scrap")' 2>/dev/null)
  [ "$answer" = "Открыть прежнюю" ] && exec open "$app" --args "$@"
else
  osascript -e 'display dialog "Сборка Scrap упала." buttons {"Лог"} default button 1 with title "Scrap"' >/dev/null 2>&1
fi
open -a Console "$log"
