# Testing the plugin marketplace locally

A manual end-to-end check of the experimental marketplace on macOS or
Linux: a plugin-enabled agent, the sandboxed plugin host, the sample plugin
in a local registry signed with throwaway keys, and the configuration app
with the Plugins page. The automated version of the same flow is
`apps/agent/tests/plugins_e2e.rs`, and over HTTPS
`apps/agent/tests/plugins_registry_https.rs` ([agent-integration.md](agent-integration.md#tests));
through the app's UI and commands it is [e2e.md](e2e.md).
Section 5 serves the same registry over HTTPS.

The app's side is **off by default**: without `VITE_DECKPILOT_PLUGINS=1`
it has no Plugins page. An agent built with the `plugins` feature has
plugins on unless started with `--disable-plugins` (or
`DECKPILOT_ENABLE_PLUGINS=0`); one built without the feature, or turned
off, refuses every plugin request (`pluginsUnavailable`). Plugins run on
macOS, Windows and Linux.

Needs: the Rust toolchain, `pnpm install` done, `jq`. Run everything from
the repository root. Every shell used below starts with these variables:
`$REG` is the registry working directory (`site/v1/`, `releases/`),
`$KEYS` holds the secret keys, the package sources and the trusted roots
(never inside `$REG`), `$DATA` is a throwaway data directory for the agent
and the app.

```sh
export REPO=$PWD SCRATCH=${TMPDIR:-/tmp}/deckpilot-plugins-test
export REG=$SCRATCH/registry KEYS=$SCRATCH/keys DATA=$SCRATCH/data
```

## 1. Build

```sh
cargo build -p deckpilot-agent --features plugins
cargo build -p deckpilot-plugin-registry --features publish --bin deckpilot-registry
(cd plugins/runtime &&
  rustup target add wasm32-unknown-unknown &&   # once, for its pinned toolchain
  cargo build --locked -p deckpilot-plugin-host --bin deckpilot-plugin-host &&
  cargo build --locked -p hello-weather --target wasm32-unknown-unknown --profile guest)
```

This gives `target/debug/deckpilot-agent`, `target/debug/deckpilot-registry`,
`plugins/runtime/target/debug/deckpilot-plugin-host` and the sample's module
`plugins/runtime/target/wasm32-unknown-unknown/guest/hello_weather.wasm`.

## 2. A local registry

The same sequence as `Registry::create` in the e2e test, in a shell you
keep open for step 4 (it defines `publish`):

```sh
R=$REPO/target/debug/deckpilot-registry
SAMPLE=$REPO/plugins/examples/hello-weather
WASM=$REPO/plugins/runtime/target/wasm32-unknown-unknown/guest/hello_weather.wasm
mkdir -p "$REG" "$KEYS"

for role in root index timestamp; do
  $R keys generate --role $role --out "$KEYS/$role.key" --no-password
done
$R keyset init --dir "$REG" --root "$KEYS/root.key.pub" \
  --index "$KEYS/index.key.pub" --timestamp "$KEYS/timestamp.key.pub"
$R keyset sign --dir "$REG" --key "$KEYS/root.key"
$R publisher --dir "$REG" --namespace acme --github-id 1001 --login acme --verified
# The roots the agent trusts: a copy apart from the registry (the tool
# keeps using $REG/roots.json).
cp "$REG/roots.json" "$KEYS/roots.json"
cat > "$KEYS/plugin.json" <<'JSON'
{"id": "acme.hello-weather", "name": "Hello Weather", "summary": "Sample plugin",
 "description": "Greets someone and shows the temperature of a city.",
 "license": "Apache-2.0", "homepage": "https://acme.example/plugins",
 "source_repository": "acme/plugins", "categories": ["other"]}
JSON

# publish VERSION [extra jq filter for manifest.json]
publish() {
  v=$1 src=$KEYS/src/$1
  mkdir -p "$src/assets"
  cp "$SAMPLE/LICENSE" "$src/"
  cp "$SAMPLE/assets/icon-128.png" "$src/assets/"
  jq ".version = \"$v\" ${2:-}" "$SAMPLE/manifest.json" > "$src/manifest.json"
  $R package --plugin-dir "$src" --module "$WASM" --out "$KEYS/hello-$v.dpplugin"
  $R add-version --dir "$REG" --package "$KEYS/hello-$v.dpplugin" \
    --plugin-detail "$KEYS/plugin.json" --source-repository acme/plugins \
    --tag "v$v" --commit "$(printf '1%.0s' $(seq 40))" --builder 1.0.0
  $R build-index --dir "$REG" --source-commit "$(printf 'a%.0s' $(seq 40))"
  $R sign-index --dir "$REG" --key "$KEYS/index.key"
  $R timestamp --dir "$REG" --key "$KEYS/timestamp.key"
  $R verify-site --dir "$REG" --require-packages
}
publish 1.0.0
```

The last line must say `site accepted by the client: … 1 installable
version(s), 1 package(s) verified`. The release asset is
`$REG/releases/acme.hello-weather--v1.0.0/acme.hello-weather-1.0.0.dpplugin`
(byte-identical to `$KEYS/hello-1.0.0.dpplugin`). The timestamp is valid
for 7 days; after that the app shows the registry as stale until you run
the `timestamp` line again.

1.1.0 (the update with an extra permission) is published later, in step
4.6. To have it in the registry from the start, run that `publish` line
now. `rm -rf "$SCRATCH"` starts over with new keys.

## 3. Start the app and the agent

In a second shell (with the variables above). Use the environment variables rather than the flags: the app starts the
agent (on launch, and from Settings › Background Agent › Start agent) as
`deckpilot-agent` next to its own executable, which for `tauri dev` is
`target/debug/deckpilot-agent` (the build from step 1). A debug build of
the app, such as `tauri dev`, also accepts `DECKPILOT_AGENT_BIN` naming
another agent executable; a release build ignores the variable and always
starts the agent next to its own executable
([background-agent.md](../background-agent.md)). That agent inherits the
app's environment. `DECKPILOT_DATA_DIR` keeps your real configuration untouched
(a debug build, or any custom data directory, keeps its key under the
keychain service `DeckPilot Development`).

```sh
export DECKPILOT_DATA_DIR=$DATA
export DECKPILOT_PLUGIN_HOST=$REPO/plugins/runtime/target/debug/deckpilot-plugin-host
export DECKPILOT_PLUGIN_REGISTRY=$REG
export DECKPILOT_PLUGIN_REGISTRY_ROOTS=$KEYS/roots.json
(cd apps/desktop && VITE_DECKPILOT_PLUGINS=1 pnpm tauri dev)
```

The host path must be **absolute**: the host is spawned in a fixed working
directory, so a relative path fails at the first action ("the plugin host
could not be started"). An agent the app starts logs only to
`$DATA/logs/`. To watch it in a terminal, start it yourself first in a
third shell with the same exports, and add `DECKPILOT_NO_AGENT_AUTOSTART=1`
to the app's shell:

```sh
target/debug/deckpilot-agent --headless --no-usb \
  --plugin-host "$DECKPILOT_PLUGIN_HOST" \
  --plugin-registry "$REG" --plugin-registry-roots "$KEYS/roots.json"
```

It logs `plugin store opened` once the configuration is unlocked. The
flags override the variables. `make up` / `make restart` are not suited
here: they rebuild `target/debug/deckpilot-agent` without the `plugins`
feature (run step 1 again afterwards).

## 4. Walk through

Settings › Extensions › Plugins.

1. **Runtime notice.** With everything above, none. Check the variants:
   stop the agent (Settings › Background Agent › Stop agent: *agent not
   running*); start it with `--disable-plugins` or
   `DECKPILOT_ENABLE_PLUGINS=0` (*turned off*); `cargo build -p deckpilot-agent` without the feature
   (*not built with plugins*; rebuild with it afterwards). *Platform
   unsupported* shows only on a system other than macOS, Windows and Linux,
   *agent too old* only with an agent before protocol 10.
2. **Discover** lists Hello Weather with *by acme*, **Verified publisher**,
   *DeckPilot >=0.5.0 <1.0.0* and its platforms; the category filter offers
   *Other*. Its details show the description, license, homepage, source,
   the compatibility of the offered version with this computer, each
   permission with the publisher's reason, and the versions with their
   publication dates (there is no changelog: the registry has none). Until it
   is installed, *Actions* says that the registry does not list them.
   **Install 1.0.0**: the consent
   dialog lists `network:api.open-meteo.com`; check it, **Install**. Progress
   shows while installing; the plugin is then installed and disabled.
3. **Installed**: switch **Enable Hello Weather** (consent dialog). Open
   **Permissions and actions**; under **Actions**, enter a name and **Run**
   *Greet* → `Hello, <name>.`; **Run** *Current
   temperature* → the temperature of the city. This makes a real request
   to `api.open-meteo.com`.
   Installed shows the sample's icon, its DeckPilot range and platforms.
   Hello Weather declares no settings or credentials, so it has no
   **Configure** button (a plugin that declares them lists them there; the
   agent cannot store them yet).
4. Under the permissions, **Revoke** the network permission, **Run** *Current
   temperature* again: it fails with `not-permitted` (denied by the
   permission broker). *Greet* still works. **Grant** allows it again.
5. **In the editors.** Action Library › **New action**: *Hello Weather*
   (plugin) lists *Greet* and *Current temperature*. Choose *Greet*: its
   *Name* and *Style* parameters are editors; create it, then **Run** it.
   Drag it onto a key in the Layout Designer and press the key; in
   Workflows, the node palette lists it under *Hello Weather* (with
   **+ Current temperature** to create one there). Switch the plugin off:
   the picker shows both actions as unavailable with the reason, the saved
   action and the workflow node stay and show *the plugin is disabled*.
6. Switch the plugin off (disabled: actions unavailable), then restart the
   agent (Stop agent / Start agent, or Ctrl-C and rerun it). The plugin,
   its state and its grants are unchanged. Switch it on again.
7. **Publish 1.1.0** (skip if you did in section 2), in the registry shell:

   ```sh
   publish 1.1.0 '| .permissions += [{"kind": "notifications",
     "justification": "Tells you when the temperature changes a lot."}]'
   ```

   `verify-site` now reports seq 2 and 2 installable versions.
   **Updates** › **Check for updates** shows 1.1.0.
8. **Update**: 1.1.0 asks for more, so the agent stages it and the consent
   dialog shows the added `notifications` permission. Check it and
   **Update** (`approvePluginUpdate` with exactly that key): the plugin
   runs 1.1.0.
9. **Roll back to 1.0.0** (Installed): back on 1.0.0.
10. **Uninstall** (confirm): gone from Installed, back to installable in
   Discover.
11. **Declined update**: install 1.0.0 again, **Updates** › **Update**,
    then **Decline update**: the plugin stays on 1.0.0 without the new
    permission. **Cancel** instead closes the dialog and leaves the update
    staged ("Waiting for consent", **Review and update** reopens it).
12. **Install from package file…**: pick
    `$REG/releases/acme.hello-weather--v1.0.0/acme.hello-weather-1.0.0.dpplugin`
    and confirm (accepted: its bytes are what the registry pins). It is
    installed with nothing granted, and the consent dialog for enabling it
    opens with its permissions; **Cancel** leaves it installed and disabled. Any other package for
    that version, e.g. one rebuilt with a changed manifest, is refused
    (`invalidPackage`: "the package is … bytes, but the registry pinned …").

## 5. The same registry over HTTPS

The agent can also fetch the registry over HTTPS (`--plugin-registry-url`,
[agent-integration.md](agent-integration.md#registry-over-https)). It
trusts the operating system's certificate authorities only; to serve the
registry locally without touching the system trust store, build the agent
with the test hooks, which let it trust **only** a throwaway CA named by
`DECKPILOT_TEST_REGISTRY_CA` (never part of a release build):

```sh
cargo build -p deckpilot-agent --features plugins,test-hooks
```

A throwaway CA and a certificate for `127.0.0.1` (valid 7 days), kept with
the other secrets in `$KEYS`:

```sh
mkdir -p "$KEYS/tls" && cd "$KEYS/tls"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 7 \
  -subj "/CN=DeckPilot local test CA" -keyout ca.key -out ca.pem \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"
openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -subj "/CN=127.0.0.1" -keyout server.key -out server.csr
printf 'subjectAltName=IP:127.0.0.1,DNS:localhost\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n' > server.ext
openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 7 \
  -extfile server.ext -out server.pem
cd "$REPO"
```

Serve the registry directory (`site/` and `releases/`) in a shell of its
own, with Python's standard library:

```sh
cat > "$KEYS/tls/serve.py" <<'PY'
import http.server, ssl, sys
root, cert, key = sys.argv[1:4]
handler = lambda *a: http.server.SimpleHTTPRequestHandler(*a, directory=root)
httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 8443), handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(cert, key)
httpd.socket = context.wrap_socket(httpd.socket, server_side=True)
httpd.serve_forever()
PY
python3 -I "$KEYS/tls/serve.py" "$REG" "$KEYS/tls/server.pem" "$KEYS/tls/server.key"
```

`curl --cacert "$KEYS/tls/ca.pem" https://127.0.0.1:8443/site/v1/timestamp.json`
shows the timestamp. Then start the agent (or the app, step 3) with the
URLs instead of the directory:

```sh
unset DECKPILOT_PLUGIN_REGISTRY
export DECKPILOT_PLUGIN_REGISTRY_URL=https://127.0.0.1:8443/site/
export DECKPILOT_PLUGIN_REGISTRY_RELEASES_URL=https://127.0.0.1:8443/releases/
export DECKPILOT_TEST_REGISTRY_CA=$KEYS/tls/ca.pem
target/debug/deckpilot-agent --headless --no-usb \
  --plugin-host "$DECKPILOT_PLUGIN_HOST" --plugin-registry-roots "$KEYS/roots.json"
```

Discover now shows `https://127.0.0.1:8443/site/` as the source, and the
server's log shows `keys/1.json`, `keys/2.json` (404: the end of the key
set chain), `timestamp.json`, `index/<seq>.json` and, on install, the
package. Things to try:

- **Offline.** Stop the server: Check for updates still lists the plugins
  (from the verified cache), `getPluginRuntime` reports `registry.offline`
  with the reason, installed plugins run, installs fail.
- **Stale.** Without a `timestamp` re-sign for 7 days (or with
  `$R timestamp … --now <8 days ago>` after deleting
  `$REG/site/v1/timestamp.json*`), the registry turns `stale`
  (`staleReason: expired`) and installs are refused.
- **Rollback.** Copy `$REG` aside, `publish` again, refresh, then serve the
  copy: the refresh is refused (`rollback: seq … is below the stored
  seq …`), and the stored state stays.
- **Tampering.** Change a byte of `$REG/site/v1/index/<seq>.json` or of a
  release asset: refused (`IndexMismatch`, `invalidPackage`).

Without `--plugin-registry-roots` (and without roots embedded at build
time, `DECKPILOT_REGISTRY_ROOT_PUBKEYS`, which no build sets yet) the
agent fetches nothing and reports that no registry is configured; with roots but without a URL or a
directory it uses the official site (`https://knittingcode.github.io/deckpilot-plugins/`),
which only verifies against its production roots (not issued yet).

## Clean up

Stop the app and the agent, then `rm -rf "$SCRATCH"`. The configuration
key stays in the keychain under `DeckPilot Development`.
