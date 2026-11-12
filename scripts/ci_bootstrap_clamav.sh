#!/usr/bin/env bash
set -euo pipefail

with_nsis=false
if [[ $# -gt 1 || ( $# -eq 1 && "$1" != "--with-nsis" ) ]]; then
  echo "usage: ci_bootstrap_clamav.sh [--with-nsis]" >&2
  exit 2
fi
if [[ $# -eq 1 ]]; then
  with_nsis=true
fi

packages=(clamav clamav-freshclam)
if [[ "$with_nsis" == true ]]; then
  packages+=(nsis)
fi

run_apt_with_retries() {
  local attempt=1
  local exit_code=1
  local max_attempts=3
  local delay
  while (( attempt <= max_attempts )); do
    if sudo timeout --signal=TERM --kill-after=10s 180s env DEBIAN_FRONTEND=noninteractive apt-get \
      -o APT::Update::Error-Mode=any \
      -o Acquire::Retries=2 \
      -o Acquire::http::Timeout=30 \
      -o Acquire::https::Timeout=30 \
      "$@"; then
      return 0
    else
      exit_code=$?
    fi
    if (( attempt == max_attempts )); then
      break
    fi
    delay=$((attempt * 2))
    echo "apt-get $1 failed (attempt $attempt/$max_attempts); retrying in ${delay}s" >&2
    sleep "$delay"
    attempt=$((attempt + 1))
  done
  echo "apt-get $1 failed after $max_attempts bounded attempts" >&2
  return "$exit_code"
}

run_apt_with_retries update -qq
run_apt_with_retries install -y -qq --no-install-recommends "${packages[@]}"

if service_state="$(sudo systemctl is-active clamav-freshclam.service 2>&1)"; then
  if [[ "$service_state" == active ]]; then
    sudo systemctl stop clamav-freshclam.service
  elif [[ "$service_state" != inactive && "$service_state" != failed ]]; then
    echo "could not determine freshclam service state: $service_state" >&2
    exit 1
  fi
else
  service_status=$?
  if [[ "$service_state" != inactive && "$service_state" != failed ]]; then
    echo "could not determine freshclam service state (exit $service_status): $service_state" >&2
    exit "$service_status"
  fi
fi

sudo timeout --signal=TERM --kill-after=10s 600s freshclam
