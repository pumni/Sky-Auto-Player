#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
temp_root="$(mktemp -d)"
trap 'rm -rf -- "$temp_root"' EXIT
mock_bin="$temp_root/bin"
mkdir -p "$mock_bin"
export MOCK_LOG="$temp_root/commands.log"
export MOCK_APT_UPDATE_FAILS=0
export MOCK_APT_INSTALL_FAILS=0
export MOCK_SERVICE_STATE=inactive
export MOCK_FRESHCLAM_EXIT=0

cat > "$mock_bin/sudo" <<'MOCK'
#!/usr/bin/env bash
exec "$@"
MOCK
cat > "$mock_bin/timeout" <<'MOCK'
#!/usr/bin/env bash
while [[ "${1:-}" == --* ]]; do shift; done
shift
exec "$@"
MOCK
cat > "$mock_bin/apt-get" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
action=""
for arg in "$@"; do
  if [[ "$arg" == update || "$arg" == install ]]; then action="$arg"; break; fi
done
[[ -n "$action" ]] || { echo "apt action missing" >&2; exit 90; }
printf 'apt:%s:%s\n' "$action" "$*" >> "$MOCK_LOG"
count_file="${MOCK_LOG}.${action}.count"
count=0
[[ ! -f "$count_file" ]] || count="$(<"$count_file")"
count=$((count + 1))
printf '%s' "$count" > "$count_file"
if [[ "$action" == update ]]; then
  failures="$MOCK_APT_UPDATE_FAILS"
else
  failures="$MOCK_APT_INSTALL_FAILS"
fi
if (( count <= failures )); then exit 42; fi
MOCK
cat > "$mock_bin/systemctl" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == is-active && "$2" == clamav-freshclam.service ]]; then
  printf '%s\n' "$MOCK_SERVICE_STATE"
  [[ "$MOCK_SERVICE_STATE" == active ]]
elif [[ "$1" == stop && "$2" == clamav-freshclam.service ]]; then
  echo stop >> "$MOCK_LOG"
else
  echo "unexpected systemctl call" >&2
  exit 91
fi
MOCK
cat > "$mock_bin/freshclam" <<'MOCK'
#!/usr/bin/env bash
echo freshclam >> "$MOCK_LOG"
exit "$MOCK_FRESHCLAM_EXIT"
MOCK
cat > "$mock_bin/sleep" <<'MOCK'
#!/usr/bin/env bash
exit 0
MOCK
chmod +x "$mock_bin"/*
export PATH="$mock_bin:$PATH"

fail() { echo "ClamAV bootstrap harness failed: $*" >&2; exit 1; }
reset_case() {
  rm -f "$MOCK_LOG" "$MOCK_LOG".*.count
  export MOCK_APT_UPDATE_FAILS=0 MOCK_APT_INSTALL_FAILS=0 MOCK_SERVICE_STATE=inactive MOCK_FRESHCLAM_EXIT=0
}

reset_case
export MOCK_APT_UPDATE_FAILS=2 MOCK_SERVICE_STATE=active
bash "$repo_root/scripts/ci_bootstrap_clamav.sh" --with-nsis
[[ "$(cat "$MOCK_LOG.update.count")" == 3 ]] || fail "apt update did not use the bounded retry count"
[[ "$(cat "$MOCK_LOG.install.count")" == 1 ]] || fail "apt install did not run once after update recovered"
grep -Fq 'apt:install:' "$MOCK_LOG" || fail "apt install was not recorded"
grep -Fq 'clamav clamav-freshclam nsis' "$MOCK_LOG" || fail "NSIS proof package set changed"
grep -Fq 'APT::Update::Error-Mode=any' "$MOCK_LOG" || fail "APT update error mode is missing"
grep -Fxq stop "$MOCK_LOG" || fail "an active freshclam daemon was not stopped"
grep -Fxq freshclam "$MOCK_LOG" || fail "freshclam was not required"

reset_case
bash "$repo_root/scripts/ci_bootstrap_clamav.sh"
if grep -Fxq stop "$MOCK_LOG"; then fail "inactive freshclam daemon was stopped"; fi
if grep -Fq 'nsis' "$MOCK_LOG"; then fail "NSIS was installed without --with-nsis"; fi

reset_case
export MOCK_APT_INSTALL_FAILS=2
bash "$repo_root/scripts/ci_bootstrap_clamav.sh"
[[ "$(cat "$MOCK_LOG.install.count")" == 3 ]] || fail "apt install did not use the bounded retry count"
grep -Fxq freshclam "$MOCK_LOG" || fail "freshclam did not run after apt install recovered"

reset_case
export MOCK_APT_UPDATE_FAILS=99
if bash "$repo_root/scripts/ci_bootstrap_clamav.sh"; then fail "persistent apt update failure was accepted"; fi
[[ "$(cat "$MOCK_LOG.update.count")" == 3 ]] || fail "apt update retry loop was not bounded at three attempts"
if grep -Fq 'apt:install:' "$MOCK_LOG" || grep -Fxq freshclam "$MOCK_LOG"; then
  fail "install or scanner refresh ran after apt update failed"
fi

reset_case
export MOCK_APT_INSTALL_FAILS=99
if bash "$repo_root/scripts/ci_bootstrap_clamav.sh"; then fail "persistent apt install failure was accepted"; fi
[[ "$(cat "$MOCK_LOG.install.count")" == 3 ]] || fail "apt install retry loop was not bounded at three attempts"
if grep -Fxq freshclam "$MOCK_LOG"; then fail "freshclam ran after apt install failed"; fi

reset_case
export MOCK_SERVICE_STATE=activating
if bash "$repo_root/scripts/ci_bootstrap_clamav.sh"; then fail "unknown freshclam service state was accepted"; fi
if grep -Fxq freshclam "$MOCK_LOG"; then fail "freshclam ran after service state could not be established"; fi

reset_case
export MOCK_FRESHCLAM_EXIT=7
if bash "$repo_root/scripts/ci_bootstrap_clamav.sh"; then fail "freshclam failure was accepted"; fi
grep -Fxq freshclam "$MOCK_LOG" || fail "freshclam failure case did not invoke freshclam"

echo "ClamAV apt/freshclam bootstrap positive and failure-injection tests: PASS"
