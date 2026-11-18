#!/usr/bin/env bash
# Starts an installed DeckPilot with an isolated profile (Linux, macOS):
#
#   tools/release-smoke/session.sh <label> launch <desktop executable>
#   tools/release-smoke/session.sh <label> gui    <desktop executable>
#
# The profile is a new directory under $SMOKE_WORK: HOME, the XDG
# directories and DECKPILOT_DATA_DIR point into it.
# - Linux: under Xvfb, in a private D-Bus session with its own GNOME
#   Keyring, whose keyring files live in the profile: the system key the
#   agent creates on first start never reaches another keyring.
# - macOS: a temporary keychain is made the default keychain for the run
#   and removed afterwards; the agent's key item must land there, not in
#   the login keychain.
#
# launch: the app starts, starts the agent; both must stay alive for 30 s;
#   the agent must create its encrypted database in the profile; a
#   screenshot is taken; nothing named *deckpilot* may appear in the real
#   home meanwhile.
# gui: tauri-driver (and WebKitWebDriver, Linux) drives the installed app
#   through tools/release-smoke/gui.mjs against the production Marketplace.
#
# The agent is found by its process name, deckpilot-agent (also inside an
# AppImage, where `<AppImage> --agent` execs the bundled agent).
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/release-smoke/common.sh
source "$here/common.sh"

label="${1:?usage: session.sh <label> launch|gui <desktop executable>}"
mode="${2:?}"
exe="${3:?}"
os=$(uname -s)

# Live agents only: an agent that exited stays a zombie (state Z) until
# the app reaps it, and must not count as running.
agent_pids() {
  local pid
  for pid in $(pgrep -x deckpilot-agent 2> /dev/null); do
    case "$(ps -o stat= -p "$pid" 2> /dev/null | tr -d ' ')" in
      Z* | "") ;;
      *) echo "$pid" ;;
    esac
  done
}

stop_all() {
  local pid pids
  [ -n "${desktop_pid:-}" ] && kill "$desktop_pid" 2> /dev/null
  pids=$(agent_pids)
  # shellcheck disable=SC2086
  [ -n "$pids" ] && kill $pids 2> /dev/null
  for _ in $(seq 1 20); do
    { [ -z "${desktop_pid:-}" ] || ! kill -0 "$desktop_pid" 2> /dev/null; } && [ -z "$(agent_pids)" ] && return 0
    sleep 0.5
  done
  [ -n "${desktop_pid:-}" ] && kill -9 "$desktop_pid" 2> /dev/null
  for pid in $(agent_pids); do kill -9 "$pid" 2> /dev/null; done
  return 0
}

screenshot() {
  local file="$SMOKE_OUT/$1.png"
  case "$os" in
    Linux)
      if command -v import > /dev/null; then import -window root "$file" 2> /dev/null
      else xwd -root -silent | convert xwd:- "$file" 2> /dev/null; fi ;;
    Darwin) screencapture -x "$file" 2> /dev/null ;;
  esac
  [ -s "$file" ] && basename "$file"
}

# ---------------------------------------------------------------- inside
inner() {
  local profile="$SMOKE_PROFILE"
  export HOME="$profile/home"
  export XDG_DATA_HOME="$profile/xdg-data" XDG_CONFIG_HOME="$profile/xdg-config"
  export XDG_CACHE_HOME="$profile/xdg-cache" XDG_STATE_HOME="$profile/xdg-state"
  export DECKPILOT_DATA_DIR="$profile/data"
  if [ "$os" = Linux ]; then
    export XDG_RUNTIME_DIR="$profile/run"
    # A keyring with the secrets component, unlocked, in the profile.
    echo "release-smoke" | gnome-keyring-daemon --daemonize --unlock --components=secrets > /dev/null 2>&1
    local ready=""
    for _ in $(seq 1 60); do
      if dbus-send --session --print-reply=literal --reply-timeout=2000 --dest=org.freedesktop.secrets \
        /org/freedesktop/secrets org.freedesktop.DBus.Properties.Get \
        string:org.freedesktop.Secret.Service string:Collections > /dev/null 2>&1; then ready=1; break; fi
      sleep 0.5
    done
    [ -n "$ready" ] || record "$label: credential store" fail "the private GNOME Keyring did not answer within 30 s"
  fi
  case "$mode" in
    launch) inner_launch ;;
    gui) inner_gui ;;
    *) echo "unknown mode $mode" >&2; exit 2 ;;
  esac
}

inner_launch() {
  local data="$DECKPILOT_DATA_DIR" alive_desktop=0 alive_agent=0 checks=0 shot agents
  "$exe" > "$SMOKE_OUT/$label-desktop.log" 2>&1 &
  desktop_pid=$!
  # 30 s, checked every 5 s.
  for _ in 1 2 3 4 5 6; do
    sleep 5
    checks=$((checks + 1))
    kill -0 "$desktop_pid" 2> /dev/null && alive_desktop=$((alive_desktop + 1))
    [ -n "$(agent_pids)" ] && alive_agent=$((alive_agent + 1))
  done
  agents=$(agent_pids | tr '\n' ' ')
  if [ "$alive_desktop" -eq "$checks" ]; then
    record "$label: desktop alive 30 s" pass "pid $desktop_pid alive at $alive_desktop/$checks checks"
  else
    record "$label: desktop alive 30 s" fail "alive at $alive_desktop/$checks checks; log: $(tail -n 5 "$SMOKE_OUT/$label-desktop.log" 2> /dev/null)"
  fi
  if [ "$alive_agent" -eq "$checks" ]; then
    record "$label: agent started by the app, alive 30 s" pass "pids $agents ($(ps -o args= -p "${agents%% *}" 2> /dev/null | cut -c1-120)) at $alive_agent/$checks checks"
  elif [ "$alive_agent" -gt 0 ]; then
    record "$label: agent started by the app, alive 30 s" fail "agent seen at only $alive_agent/$checks checks"
  else
    record "$label: agent started by the app, alive 30 s" fail "no deckpilot-agent process"
  fi
  # The encrypted database in the isolated data directory.
  for _ in $(seq 1 30); do [ -s "$data/deckpilot.db" ] && break; sleep 1; done
  local listing
  listing=$(cd "$data" 2> /dev/null && find . -maxdepth 2 | sed 's#^\./##' | LC_ALL=C sort | tr '\n' ' ')
  if encrypted_db "$data/deckpilot.db"; then
    record "$label: encrypted database in the isolated profile" pass "$data/deckpilot.db ($(wc -c < "$data/deckpilot.db" | tr -d ' ') bytes, no 'SQLite format 3' header); data dir: $listing"
  elif [ -s "$data/deckpilot.db" ]; then
    record "$label: encrypted database in the isolated profile" fail "$data/deckpilot.db is a plain SQLite file: $(wc -c < "$data/deckpilot.db" | tr -d ' ') bytes, header $(head -c 16 "$data/deckpilot.db" | od -An -tx1 | tr -d ' \n')"
  else
    record "$label: encrypted database in the isolated profile" fail "no $data/deckpilot.db; data dir: ${listing:-empty}; agent log: $(tail -n 3 "$data"/logs/* 2> /dev/null | tr '\n' ' ')"
  fi
  if [ "$os" = Linux ]; then
    local rings
    rings=$(find "$XDG_DATA_HOME/keyrings" -type f 2> /dev/null | sed "s#^$SMOKE_PROFILE/##" | tr '\n' ' ')
    if [ -n "$rings" ]; then
      record "$label: system key in the private keyring" pass "keyring files in the profile: $rings"
    else
      record "$label: system key in the private keyring" fail "no keyring file under $XDG_DATA_HOME/keyrings"
    fi
  fi
  shot=$(screenshot "$label-first-launch")
  if [ -n "$shot" ]; then
    record "$label: screenshot" pass "artifact file $shot"
  else
    record "$label: screenshot" fail "no screenshot could be taken"
  fi
  stop_all
  if [ -z "$(agent_pids)" ] && ! kill -0 "$desktop_pid" 2> /dev/null; then
    record "$label: stopped" pass "desktop and agent terminated"
  else
    record "$label: stopped" fail "processes left: $(agent_pids | tr '\n' ' ')"
  fi
  # The app starts the agent with its output discarded: when no database
  # was made, run the installed agent once more by hand for its stderr.
  if [ "$os" = Linux ] && [ ! -s "$data/deckpilot.db" ] && [ -n "${SMOKE_AGENT:-}" ]; then
    local diag libs
    # shellcheck disable=SC2086 # SMOKE_AGENT_ARGS: words, e.g. --agent
    diag=$(timeout 15 "$SMOKE_AGENT" ${SMOKE_AGENT_ARGS-} 2>&1 | tail -n 6)
    libs=$(ldconfig -p | grep -i -E 'appindicator' | sed 's/ =>.*//' | tr -s ' \t' ' ' | tr '\n' ';')
    record "$label: agent run by hand (diagnosis)" fail "$SMOKE_AGENT ${SMOKE_AGENT_ARGS-} exited with: $(printf '%s' "$diag" | tr '\n' ' '); appindicator libraries installed: ${libs:-none}"
    stop_all
  fi
  # Logs (never the database or the token).
  mkdir -p "$SMOKE_OUT/$label-agent-logs"
  cp -R "$data/logs/." "$SMOKE_OUT/$label-agent-logs/" 2> /dev/null || true
}

inner_gui() {
  local port native log="$SMOKE_OUT/$label-tauri-driver.log" driver="${TAURI_DRIVER:-tauri-driver}" args=()
  port=$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  native=$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  [ -n "${NATIVE_DRIVER:-}" ] && args=(--native-driver "$NATIVE_DRIVER")
  "$driver" --port "$port" --native-port "$native" "${args[@]}" > "$log" 2>&1 &
  local server=$!
  local up=""
  for _ in $(seq 1 120); do
    curl -fs "http://127.0.0.1:$port/status" > /dev/null 2>&1 && { up=1; break; }
    sleep 0.5
  done
  if [ -z "$up" ]; then
    record "$label: tauri-driver" not-covered "tauri-driver did not answer on port $port: $(tail -n 5 "$log" | tr '\n' ' ')"
  else
    node "$here/gui.mjs" --port "$port" --app "$exe" --label "$label" --out "$SMOKE_OUT" 2>&1 | tee "$SMOKE_OUT/$label-gui.log"
  fi
  kill "$server" 2> /dev/null
  wait "$server" 2> /dev/null
  stop_all
  mkdir -p "$SMOKE_OUT/$label-agent-logs"
  cp -R "$DECKPILOT_DATA_DIR/logs/." "$SMOKE_OUT/$label-agent-logs/" 2> /dev/null || true
}

if [ -n "${SMOKE_IN_SESSION:-}" ]; then
  inner
  exit 0
fi

# --------------------------------------------------------------- outside
: "${SMOKE_WORK:?SMOKE_WORK is not set}"
real_home="$HOME"
profile="$SMOKE_WORK/profile-$label"
rm -rf "$profile"
mkdir -p "$profile"/{home,xdg-data,xdg-config,xdg-cache,xdg-state,run,data}
chmod 700 "$profile/run" "$profile/data"
marker="$SMOKE_WORK/.marker-$label"
touch "$marker"
sleep 1

case "$os" in
  Linux)
    SMOKE_IN_SESSION=1 SMOKE_PROFILE="$profile" \
      xvfb-run -a -s "-screen 0 1440x900x24" dbus-run-session -- bash "$0" "$@"
    ;;
  Darwin)
    # The Keychain looks for the default keychain under $HOME: with HOME
    # in the profile (run 44272472526: "A default keychain could not be
    # found") it must exist there. The keychain is the profile's login
    # keychain, and is also made the user's default for the run in case
    # the lookup does not follow HOME; both are restored afterwards.
    keychain="$profile/home/Library/Keychains/login.keychain-db"
    mkdir -p "$profile/home/Library/Keychains" "$profile/home/Library/Preferences"
    original_default=$(security default-keychain -d user | tr -d ' "')
    original_list=$(security list-keychains -d user | tr -d '"' | xargs)
    security create-keychain -p "" "$keychain"
    security set-keychain-settings "$keychain"
    security unlock-keychain -p "" "$keychain"
    HOME="$profile/home" security list-keychains -d user -s "$keychain"
    HOME="$profile/home" security default-keychain -d user -s "$keychain"
    # shellcheck disable=SC2086 # the list is whitespace-separated paths
    security list-keychains -d user -s "$keychain" $original_list
    security default-keychain -d user -s "$keychain"
    SMOKE_IN_SESSION=1 SMOKE_PROFILE="$profile" bash "$0" "$@"
    # With DECKPILOT_DATA_DIR set the agent uses the service name
    # "DeckPilot Development" (apps/agent/src/service/security.rs); an
    # installed DeckPilot's is "DeckPilot".
    if [ "$mode" = launch ]; then
      found=""
      for service in "DeckPilot Development" DeckPilot; do
        if security find-generic-password -s "$service" "$keychain" > /dev/null 2>&1; then
          found="service '$service', $(security find-generic-password -s "$service" "$keychain" 2>&1 | grep -m1 '"acct"' | tr -s ' ')"
          break
        fi
      done
      if [ -n "$found" ]; then
        record "$label: system key in the temporary keychain" pass "item in $(basename "$keychain"): $found"
      else
        record "$label: system key in the temporary keychain" fail "no DeckPilot item in the temporary default keychain"
      fi
    fi
    login="$real_home/Library/Keychains/login.keychain-db"
    leaked=""
    for service in "DeckPilot Development" DeckPilot; do
      if [ -f "$login" ] && security find-generic-password -s "$service" "$login" > /dev/null 2>&1; then
        leaked="$leaked '$service'"
        security delete-generic-password -s "$service" "$login" > /dev/null 2>&1 || true
      fi
    done
    if [ -n "$leaked" ]; then
      record "$label: login keychain untouched" fail "items of service$leaked appeared in the runner's login keychain (removed)"
    else
      record "$label: login keychain untouched" pass "no DeckPilot item in $login"
    fi
    security default-keychain -d user -s "$original_default"
    # shellcheck disable=SC2086
    security list-keychains -d user -s $original_list
    security delete-keychain "$keychain" 2> /dev/null || true
    ;;
  *) echo "unsupported OS $os" >&2; exit 2 ;;
esac

# Nothing of DeckPilot in the real home (the workspace, the toolchains and
# the profile itself excluded).
leaked=$(find "$real_home" -maxdepth 5 \( -path "$real_home/work" -o -path "$real_home/.cargo" -o -path "$real_home/.rustup" \
  -o -path "$SMOKE_WORK" -o -path "$real_home/Library/Keychains" \) -prune -o -newer "$marker" -iname '*deckpilot*' -print 2> /dev/null | head -n 20)
# On macOS WebKit (Library/WebKit, Library/Caches) and the preferences
# daemon (Library/Preferences) use the account's home directory whatever
# HOME says, so those cannot be isolated by HOME: reported as not covered
# and removed (they were created by this run on a throwaway runner).
system=""
other=""
while IFS= read -r path; do
  [ -n "$path" ] || continue
  case "$os:$path" in
    "Darwin:$real_home/Library/WebKit/"* | "Darwin:$real_home/Library/Caches/"* | "Darwin:$real_home/Library/Preferences/"* | "Darwin:$real_home/Library/HTTPStorages/"*)
      system="$system $path" ;;
    *) other="$other $path" ;;
  esac
done <<< "$leaked"
if [ -n "$other" ]; then
  record "$label: real home untouched" fail "new paths in the real home:$other"
elif [ -n "$system" ]; then
  record "$label: real home untouched" not-covered "nothing else appeared, but WebKit and the preferences daemon ignore HOME and created:$system (removed)"
else
  record "$label: real home untouched" pass "no new *deckpilot* path in $real_home (depth 5) during the run"
fi
if [ -n "$system" ]; then
  # shellcheck disable=SC2086 # paths without spaces, collected above
  rm -rf $system
fi
exit 0
