#!/usr/bin/env bash
set -euo pipefail

source_root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "${work:?}"' EXIT

sed -n '/^ui_test_classes() {/,/^action_coverage() {/p' "$source_root/scripts/build-ios.sh" |
  sed '$d' >"$work/functions.sh"
cat >"$work/run.sh" <<'EOF'
set -euo pipefail
source "$1"
IOS_DIR="$2"
BUILD_DIR="$3"
DEVICE=phone
UI_SHARD=""
ONLY=()
die() { echo "$*" >&2; exit 1; }
run_test_plan() { printf '%s\n' "$*"; }
report_retried_tests() { return 0; }
coverage_summarize() { return 0; }
case "$4" in
  ui) action_smoke; action_ui ;;
  test) action_smoke; action_test ;;
  filter)
    ONLY=(-only-testing:NgTermUITests/SettingsTests)
    action_smoke
    action_ui
    ;;
  wrong-class)
    UI_SMOKE_TESTS=(FirstRunTests/testLaunchesOnTheFakeCore)
    action_smoke
    ;;
  missing-class)
    UI_SMOKE_TESTS=(MissingTests/testLaunchesOnTheFakeCore)
    action_smoke
    ;;
  missing-method)
    UI_SMOKE_TESTS=(LaunchTests/testMissing)
    action_smoke
    ;;
esac
EOF

run() {
  bash "$work/run.sh" "$work/functions.sh" "$source_root/mobile/ios" "$work/build" "$1"
}
smoke="$(run ui)"
smoke="${smoke%%$'\n'*}"
[[ "$smoke" == "UI -retry-tests-on-failure -test-iterations 2 "* ]]
[[ "$(tr ' ' '\n' <<<"$smoke" | grep -c '^-only-testing:NgTermUITests/')" == 4 ]]
full="UI -retry-tests-on-failure -test-iterations 2"
[[ "$(run ui)" == "$smoke"$'\n'"$full" ]]
echo "ok: smoke ui runs the full UI plan after the smoke set"
[[ "$(run test)" == "$smoke"$'\nUnit\n'"$full" ]]
echo "ok: smoke test runs the Unit plan and the full UI plan after the smoke set"
[[ "$(run filter)" == "$smoke"$'\n'"$full -only-testing:NgTermUITests/SettingsTests" ]]
echo "ok: smoke preserves an existing UI filter"
for invalid in wrong-class missing-class missing-method; do
  if run "$invalid" >"$work/out" 2>"$work/err"; then
    echo "FAIL: $invalid must fail" >&2
    exit 1
  fi
  [[ ! -s "$work/out" ]]
  grep -q 'the smoke test .* is not in mobile/ios/NgTermUITests' "$work/err"
  echo "ok: $invalid stops before the UI plan"
done
