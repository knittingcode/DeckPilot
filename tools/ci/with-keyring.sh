#!/usr/bin/env bash
# Runs a command against a real, unlocked GNOME Keyring (Secret Service) in
# a private D-Bus session, for the credential store tests on Linux CI:
#
#   tools/ci/with-keyring.sh <command> [<args>...]
#
# Starts its own D-Bus session (dbus-run-session) and keyring daemon, and
# waits until the Secret Service answers before running the command; a
# daemon that is not ready made tests fail with "NoReply: Message
# recipient disconnected". If the command then fails because the Secret
# Service could not be reached (not because of a test assertion), it is run
# once more in a fresh session; any other failure is final.
set -euo pipefail

[ "$#" -gt 0 ] || { echo "usage: with-keyring.sh <command> [<args>...]" >&2; exit 2; }

if [ -z "${WITH_KEYRING_SESSION:-}" ]; then
  log="$(mktemp)"
  trap 'rm -f "$log"' EXIT
  for attempt in 1 2; do
    set +e
    WITH_KEYRING_SESSION=1 dbus-run-session -- "$0" "$@" 2>&1 | tee "$log"
    status=${PIPESTATUS[0]}
    set -e
    [ "$status" -eq 0 ] && exit 0
    # Retry only when the keyring itself was unreachable.
    if [ "$attempt" -eq 1 ] && grep -q -E 'Unreachable \{ store: "Secret Service"|org\.freedesktop\.DBus\.Error\.(NoReply|ServiceUnknown)|Secret Service did not answer' "$log"; then
      echo "::warning::the Secret Service was unreachable; running once more in a fresh D-Bus session"
      continue
    fi
    exit "$status"
  done
fi

# Inside the D-Bus session: an unlocked keyring with the secrets component.
echo "ci-keyring" | gnome-keyring-daemon --daemonize --unlock --components=secrets > /dev/null

# Ready when the Secret Service answers a property read (at most 30 s).
ready=""
for _ in $(seq 1 60); do
  if dbus-send --session --print-reply=literal --reply-timeout=2000 \
    --dest=org.freedesktop.secrets /org/freedesktop/secrets \
    org.freedesktop.DBus.Properties.Get \
    string:org.freedesktop.Secret.Service string:Collections > /dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.5
done
[ -n "$ready" ] || { echo "the Secret Service did not answer within 30 s" >&2; exit 1; }
echo "Secret Service ready" >&2

exec "$@"
