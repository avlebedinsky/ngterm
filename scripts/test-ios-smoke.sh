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
  expected) printf ' -only-testing:NgTermUITests/%s' "${UI_SMOKE_TESTS[@]}" ;;
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
failures=0
check() {
  local name="$1"
  shift
  if "$@"; then
    printf 'ok: %s\n' "$name"
  else
    printf 'FAIL: %s\n' "$name" >&2
    failures=$((failures + 1))
  fi
}
full="UI -retry-tests-on-failure -test-iterations 2"
smoke="$full$(run expected)"
status=0
out="$(run ui)" || status=$?
check "smoke ui succeeds" test "$status" -eq 0
check "smoke ui runs the configured set, then the full UI plan" \
  test "$out" = "$smoke"$'\n'"$full"
status=0
out="$(run test)" || status=$?
check "smoke test succeeds" test "$status" -eq 0
check "smoke test runs the configured set, then the Unit and full UI plans" \
  test "$out" = "$smoke"$'\nUnit\n'"$full"
status=0
out="$(run filter)" || status=$?
check "smoke preserves an existing UI filter and succeeds" test "$status" -eq 0
check "smoke preserves an existing UI filter" \
  test "$out" = "$smoke"$'\n'"$full -only-testing:NgTermUITests/SettingsTests"
for invalid in wrong-class missing-class missing-method; do
  status=0
  run "$invalid" >"$work/out" 2>"$work/err" || status=$?
  check "$invalid returns a failure" test "$status" -ne 0
  check "$invalid stops before the UI plan" test ! -s "$work/out"
  check "$invalid reports the invalid smoke test" \
    grep -q 'the smoke test .* is not in mobile/ios/NgTermUITests' "$work/err"
done
if ((failures)); then
  echo "$failures check(s) failed" >&2
  exit 1
fi
echo "all checks passed"
