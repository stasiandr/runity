#!/usr/bin/env bash
# The engine's acceptance, one entry for agents and for the test fleet's CI
# (the plan: github.com/stasiandr/not-divinity docs/TEST_FLEET.md). The same
# steps as .github/workflows/ci.yml, runnable anywhere.
#
#   ./check.sh fast      cargo nextest run (the fast tier)             (every change)
#   ./check.sh full      clippy, the ci tier with and without audio, doctests,
#                        the bare and web builds, the examples         (every push)
#   ./check.sh nightly   the daily tier (every test)
#
# Every step runs even after one fails; the exit code is the number of failed
# steps. Logs: target/check/<mode>/<n>-<step>.log and summary.txt.
# CHECK_ONLY="clippy tests" runs the named steps only.
#
# CHECK_SLOW=1 says the machine is a weak one (the fleet's N100 box exports
# it): the one wall-clock budget measured for a developer's machine —
# scrap-studio's a_big_scene_stays_quick — is held at three times its length
# there, and says so in its output. Nothing is skipped.
#
# `fmt` is a step of its own and not in `full` yet: the tree is not formatted
# (290 files on 2026-11-12). Put it into `full` when it is.
set -uo pipefail
cd "$(dirname "$0")"
MODE="${1:-fast}"
OUT="target/check/$MODE"
rm -rf "$OUT"; mkdir -p "$OUT"

case "$MODE" in
  fast)    PLAN="tests_fast" ;;
  full)    PLAN="clippy clippy_shell tests doctests tests_audio bare web_core examples" ;;
  nightly) PLAN="tests_daily" ;;
  *) echo "usage: ./check.sh fast|full|nightly"; exit 2 ;;
esac
[ -n "${CHECK_ONLY:-}" ] && PLAN="$CHECK_ONLY"
TOTAL=0; for _ in $PLAN; do TOTAL=$((TOTAL + 1)); done

fmt()          { cargo fmt --all -- --check; }
clippy()       { cargo clippy --workspace --all-targets -- -D warnings; }
clippy_shell() { cargo clippy --workspace --all-targets --features desktop-shell -- -D warnings; }
tests_fast()   { cargo nextest run --workspace; }
tests()        { cargo nextest run --workspace -P ci; }
tests_audio()  { cargo nextest run --workspace -P ci --features scrap/audio; }
tests_daily()  { cargo nextest run --workspace -P daily; }
doctests()     { cargo test --workspace --doc; }
bare()         { cargo check -p scrap-engine --no-default-features; }
web_core()     { cargo check -p scrap-core --target wasm32-unknown-unknown; }
examples() {
  cargo run -q -p scrap-cli -- check examples/showreel &&
    cargo run -q -p scrap-cli -- check examples/graphs
}

N=0; FAILED=(); START=$(date +%s)
for step in $PLAN; do
  N=$((N + 1)); t0=$(date +%s)
  printf '%d/%d %s …\n' "$N" "$TOTAL" "$step"
  "$step" > "$OUT/$N-$step.log" 2>&1; rc=$?
  dt=$(($(date +%s) - t0))
  if [ $rc = 0 ]; then word=ok; else word=FAILED; FAILED+=("$step"); fi
  printf '%d/%d %s %s %ds\n' "$N" "$TOTAL" "$step" "$word" "$dt" | tee -a "$OUT/summary.txt"
  if [ $word = FAILED ]; then tail -n 25 "$OUT/$N-$step.log" | sed 's/^/    /'; fi
done
{
  echo "mode $MODE, commit $(git rev-parse --short HEAD 2>/dev/null)$(git diff --quiet 2>/dev/null || echo +dirty), $(uname -sm), $(($(date +%s) - START)) s"
  if [ ${#FAILED[@]} -gt 0 ]; then echo "RED: ${FAILED[*]}"; else echo "GREEN"; fi
} | tee -a "$OUT/summary.txt"
exit ${#FAILED[@]}
