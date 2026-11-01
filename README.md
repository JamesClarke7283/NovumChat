# NovumChat

A multi-model AI chat client for desktop, mobile and the browser. Talk to
Ollama and any OpenAI-compatible endpoint, ask several models at once, let them
use tools on your machine under rules you choose, and keep your history in a
local database that can be encrypted.

Built in Rust: a [Tauri 2](https://tauri.app/) shell, a [Leptos](https://leptos.dev/)
UI compiled to WebAssembly, and a shared core crate.

## Features

- **Any number of providers.** Native [Ollama](https://ollama.com/) (via
  `ollama-rs`) and OpenAI-compatible APIs (via `async-openai`): OpenAI,
  OpenRouter, LM Studio, vLLM, llama.cpp, Groq and others. Models are
  discovered automatically.
- **Streaming everywhere.** Replies, reasoning ("thinking") and tool calls stream
  token by token, with time and tokens/sec per reply.
- **Ensembles.** Send one prompt to several models and compare their answers
  side by side (tabs on phones), then optionally have a model combine them.
- **Rich replies.** Markdown, LaTeX (`$…$`, `$$…$$`, `\(…\)`, `\[…\]`,
  `<script type="math/tex">`, chemistry via mhchem), syntax highlighting,
  Mermaid diagrams, and **live inline apps**: HTML/SVG/JavaScript runs in a
  sandboxed frame with network access off by default.
- **Attachments.** Images (sent to vision models), text and code files, and
  PDFs (text is extracted). Paste, drag and drop, or pick files.
- **Built-in tools.** `bash`, `read_file`, `write_file`, `edit_file`,
  `list_dir`, `search_files`, `web_search` and `fetch_url`, governed by
  permission levels with safe defaults and a YOLO mode.
- **MCP servers.** Add stdio or streamable-HTTP servers in the same
  `[mcp_servers]` format as the OpenAI Codex `config.toml`, or import an
  existing Codex config.
- **Web search.** DuckDuckGo (no API key), SearXNG and Brave Search.
- **Optional encryption.** AES-256-GCM for messages and attachments, with a
  key derived from your passphrase by Argon2id.
- **Conversation tree.** Edit any message, regenerate with the same or another
  model, and switch between branches.
- **Made for every screen.** Auto, light and dark themes, six accent colours,
  and a layout tuned for phones (drawer navigation, swipeable ensemble tabs).

## Quick start

You need [Rust](https://rustup.rs/) 1.94 or newer, Node.js 20 or newer,
[Trunk](https://trunkrs.dev/) and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
npm ci

npm run dev          # desktop app with hot reload
```

On first start NovumChat writes a commented `config.toml` with Ollama
(`http://localhost:11434`) and OpenAI (key from `OPENAI_API_KEY`) already set
up. Add more providers in **Settings → Providers**, or edit the file.

### Run it in a browser

The same binary can serve the UI over HTTP, which is handy on a home server or
for using it from a phone on your network:

```sh
npm run serve                                # builds the UI, then serves it on 127.0.0.1:8484
novumchat --serve 0.0.0.0:8484               # an installed build; prints a URL with an access token
novumchat --serve --static path/to/dist      # serve UI files from a folder
```

Binding to anything other than loopback requires an access token. It is
generated on first use, saved in `config.toml` and printed as
`http://host:8484/?token=…`; open that link once and the browser keeps it in a
cookie. Use a reverse proxy with TLS if you expose the port beyond your LAN.

## Where your data lives

Locations follow each platform's conventions (via the `directories` crate, the
Rust counterpart of Python's `platformdirs`):

| OS | `config.toml` | `novumchat.db` (SQLite) |
| --- | --- | --- |
| Linux | `~/.config/novumchat/` | `~/.local/share/novumchat/` |
| macOS | `~/Library/Application Support/com.NovumChat.NovumChat/` | same folder |
| Windows | `%APPDATA%\NovumChat\NovumChat\config\` | `%APPDATA%\NovumChat\NovumChat\data\` |
| Android, iOS | the app's private storage | the app's private storage |

Set `NOVUMCHAT_CONFIG_DIR` and `NOVUMCHAT_DATA_DIR` to use other folders. Tools
work in `~/NovumChat` unless you set `tools.workspace`.

## Configuration

Everything in **Settings** is stored in `config.toml`, and the file can be edited
by hand or in **Settings → config.toml**. A shortened example:

```toml
[general]
default_model = { provider = "ollama", model = "llama3.2" }
auto_title = true
max_tool_rounds = 16

[appearance]
theme = "auto"            # auto | light | dark
accent = "aurora"         # aurora | ocean | sunset | forest | rose | mono

[[providers]]
id = "ollama"
name = "Ollama"
kind = "ollama"
base_url = "http://localhost:11434"

[[providers]]
id = "openrouter"
name = "OpenRouter"
kind = "openai"           # any OpenAI-compatible endpoint
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"   # or api_key = "…"
# models = ["extra/model-id"]        # added to the discovered list
# headers = { "X-Title" = "NovumChat" }

[ensemble]
synthesize = true
synthesizer = { provider = "openrouter", model = "openai/gpt-4.1" }
default_models = [
  { provider = "ollama", model = "llama3.2" },
  { provider = "openrouter", model = "anthropic/claude-sonnet-4.5" },
]

[tools]
permission = "ask"        # read_only | ask | auto_edit | yolo
# workspace = "/home/me/projects"
bash_timeout_secs = 120

[search]
engine = "duckduckgo"     # duckduckgo | searxng | brave
searxng_url = "http://localhost:8888"
brave_api_key_env = "BRAVE_API_KEY"

[mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/me/projects"]

[mcp_servers.context7]
url = "https://mcp.context7.com/mcp"
http_headers = { "CONTEXT7_API_KEY" = "…" }
```

The default file written on first start documents every section.

### MCP servers

`[mcp_servers.<name>]` tables use the OpenAI Codex format, so entries can be
copied between the two (or use **Settings → MCP → Import from Codex**, which reads
`~/.codex/config.toml`).

| Key | Meaning |
| --- | --- |
| `command`, `args`, `env`, `env_vars`, `cwd` | Start a local server over stdio. `env_vars` forwards variables from NovumChat's own environment. |
| `url`, `bearer_token_env_var`, `http_headers`, `env_http_headers` | Connect to a streamable-HTTP server. |
| `enabled`, `startup_timeout_sec`, `tool_timeout_sec` | Turn a server off, or change its time limits. |
| `enabled_tools`, `disabled_tools` | Expose only some tools, or hide some. |
| `trusted` | NovumChat extension: run this server's tools without asking. In read-only mode only its read-only tools run. |

Tools appear to models as `mcp__<server>__<tool>`. Each call shows its result
inline in the chat.

### Tools and permissions

Every tool call is classified by what it can do (read, search, fetch, write,
execute, or call an MCP tool) and checked against the permission level, which
you can change per chat from the composer:

| Level | Reads in workspace | Web search | Fetch a URL | File edits | Shell commands | MCP tools |
| --- | --- | --- | --- | --- | --- | --- |
| Read only | yes | yes | ask | no | no | ask if read-only, else no |
| **Ask** (default) | yes | yes | ask | ask | ask (except safe commands) | ask |
| Auto-edit | yes | yes | yes | yes, inside the workspace | ask (except safe commands) | ask |
| YOLO | yes | yes | yes | yes | yes | yes |

- Reading or changing anything outside the workspace asks first (below YOLO;
  read-only mode never changes files).
- Approval cards show the exact command, or a diff of the change, with
  **Allow once**, **Always allow in this chat** and **Deny**.
- `allowed_commands` lists read-only commands that run without asking
  (`ls`, `cat`, `grep`, `git status`, …). A listed command still asks if it
  chains or redirects (`;`, `|`, `>`, `$( )`), refers to files outside the
  workspace (absolute paths, `..`, `~`, `$HOME`, symlinks), or uses an option
  that writes or runs programs (`find -delete`, `rg --pre`, `git branch -D`).
- `denied_commands` (such as `rm -rf /` and `mkfs`) are refused even in YOLO
  mode.
- Changes to files that can run commands later (`.git/` hooks and config,
  `.envrc`) always ask, even in auto-edit mode.
- MCP servers marked `trusted = true` run without asking.

### Web search

| Engine | Setup |
| --- | --- |
| DuckDuckGo | Nothing: no account or key. Results are cached for 10 minutes. If DuckDuckGo rate-limits your IP, switch engines for a while. |
| SearXNG | Point `searxng_url` at an instance with the JSON format enabled (`search.formats: [html, json]` in its `settings.yml`). |
| Brave Search | Put an API key in the environment variable named by `brave_api_key_env` (default `BRAVE_API_KEY`), or set `brave_api_key`. |

Toggle **Web** in the composer to offer search to the model, and test the engine
in **Settings → Search**. Results appear as numbered sources under the reply.

### Encryption

**Settings → Security → Encrypt chats** encrypts chat titles and settings,
messages (including reasoning and tool results) and attachments (names and
contents) in the database:

- A random 256-bit data key encrypts each value with AES-256-GCM.
- The data key is wrapped by a key derived from your passphrase with Argon2id
  (64 MiB memory, 3 passes).
- After encryption the database is vacuumed, and SQLite's `secure_delete` is on,
  so old plaintext pages are overwritten.
- Changing the passphrase re-wraps the data key; disabling encryption decrypts
  everything in place.

NovumChat asks for the passphrase on start and can be locked at any time. There
is no recovery: if you forget the passphrase, the encrypted chats cannot be read.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| Enter / Shift+Enter | Send / new line |
| Ctrl+Shift+O | New chat |
| Ctrl+K | Search chats |
| Ctrl+, | Settings |
| Esc | Close the open panel or stop generating |

Use Cmd instead of Ctrl on macOS.

## Development

```sh
npm run dev                         # desktop app, UI hot reload on :1420
npm run serve                       # web mode on 127.0.0.1:8484
npm run mock-llm                    # fake Ollama + OpenAI server on 127.0.0.1:11999
trunk build                         # the backend embeds the UI, so build it once first
cargo test                          # core + backend
cargo test -p novumchat-core --features markdown
npm test                            # build script tests
```

The mock LLM needs no GPU or API key. It streams realistic replies and can
exercise reasoning, tool calls, MCP, maths, diagrams, live apps and errors
depending on keywords in your prompt. Add it as a provider:

```toml
[[providers]]
id = "mock"
name = "Mock"
kind = "openai"            # or "ollama" with base_url = "http://127.0.0.1:11999"
base_url = "http://127.0.0.1:11999/v1"
```

### Project layout

| Crate | What it does |
| --- | --- |
| `novumchat-core` | Types shared by the UI and backend: config, RPC messages, events, the permission policy, the conversation tree and the Markdown/LaTeX renderer. |
| `novumchat-backend` | The Tauri app and the `--serve` web server: providers, streaming, ensembles, tools, MCP, search, SeaORM + SQLite storage and encryption. |
| `novumchat-frontend` | The Leptos UI, compiled to WebAssembly with Trunk. |

The UI talks to the backend through one RPC entry point (a Tauri command on
desktop and mobile, `POST /api/rpc/{method}` in web mode) and receives streamed
updates as events (Tauri events or server-sent events).

## Building

```sh
npm run build                              # desktop packages, plus Android (and iOS on macOS)
TAURI_BUILD_MOBILE=false npm run build     # desktop only
```

`npm run build` runs `scripts/build.mjs`, which builds the UI in release mode
and then the Android APK/AAB (and an unsigned iOS IPA on macOS) before Tauri
packages the desktop app. In PowerShell, set `$env:TAURI_BUILD_MOBILE = 'false'`
for a desktop-only build.

### Android

Install Android Studio, its command-line tools, and Java 21 using the
[Android prerequisites](https://v2.tauri.app/start/prerequisites/#android).
Set `JAVA_HOME` to your Java installation and `ANDROID_HOME` to your Android SDK.
With `sdkmanager` on your PATH, install the versions used by CI:

```sh
sdkmanager "platform-tools" "platforms;android-37.0" "build-tools;37.0.0" "ndk;28.2.13676358"
export NDK_HOME="$ANDROID_HOME/ndk/28.2.13676358"

# Create this app's private release signing key (once)
npm run android:signing:init

# Generate the native project and install Android Rust targets (once)
npm run tauri -- android init

# Build release APK and AAB for ARM64 devices
npm run build:android -- --ci --target aarch64 --apk --aab
```

The signed APK and AAB are under `novumchat-backend/gen/android/app/build/outputs/apk/`
and `novumchat-backend/gen/android/app/build/outputs/bundle/`. Install the APK on an
ARM64 device running Android 7.0 or newer; the AAB is for app distribution tools.
Android refuses to install unsigned APKs.

Keep Android signing material in the repository's `keys/` folder. The setup
command creates these private files:

```text
keys/
  release.jks        # Persistent Android release signing key
  credentials.json   # Key alias, key password and keystore password
```

The entire `keys/` directory is excluded by `.gitignore`; do not commit or share
these files. Back up both files securely: future updates must use the same key.
Running `npm run android:signing:init` again preserves an existing key.
Release builds automatically load these credentials and fail clearly if signing
is missing. Debug builds use Android's debug signing instead.

If you already have a signing key, put the keystore in `keys/` and set
`ANDROID_KEYSTORE_PATH` to its path (for example, `keys/release.jks`), along with
`ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`, and `ANDROID_STORE_PASSWORD`.
These environment variables override the generated local credentials. See
[Android signing](https://v2.tauri.app/distribute/sign/android/).

To use the same key in GitHub Actions, authenticate the GitHub CLI (`gh`) and run:

```sh
npm run android:signing:github
```

This stores `ANDROID_KEY_BASE64`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`,
and `ANDROID_STORE_PASSWORD` as Actions secrets in the current repository.
CI restores the same private keystore for each build; it never generates a new
signing key. After restoring a backup on another computer, keep both files in
that checkout's `keys/` folder to sign compatible updates.

If installation still says "App Not Installed", connect the device with USB
debugging enabled and run `adb install -r /path/to/app.apk` to see the exact
reason. A signature mismatch means an installed copy uses another key. Use its
original key to update it; uninstalling that copy removes its app data.

Android requires a positive version code. Keep the versions in
`novumchat-backend/tauri.conf.json` and `novumchat-backend/Cargo.toml` in sync with the release
tag when releasing, and increase the version for each update.

### iOS (macOS only)

Install Xcode with the iOS SDK and simulator runtime, select it as the active
developer directory, and follow the
[iOS prerequisites](https://v2.tauri.app/start/prerequisites/#ios).

```sh
brew install xcodegen cocoapods libimobiledevice

# Generate the native project and install iOS Rust targets (once)
npm run tauri -- ios init

# Build an unsigned release IPA for ARM64 iOS devices
npm run build:ios -- --ci --target aarch64 --no-sign
```

The IPA is under `novumchat-backend/gen/apple/build/arm64/`. This compiles without an
Apple development team or signing secrets. Device installation, TestFlight,
and App Store distribution require [iOS signing](https://v2.tauri.app/distribute/sign/ios/).

### GitHub Actions

The `release` workflow builds all eight platform/architecture combinations:

| Target | Runner | Release artifacts |
| --- | --- | --- |
| Linux x86_64 and ARM64 | Ubuntu | AppImage, DEB and RPM |
| Windows x86_64 and ARM64 | Windows | MSI |
| macOS Intel x86_64 and Apple Silicon ARM64 | macOS | DMG and app archives |
| Android ARM64 | Ubuntu | Signed release APK and AAB |
| iOS ARM64 | macOS with Xcode | Unsigned release IPA |

Run it manually on a branch from the Actions tab to download workflow artifacts,
or push a `v*` tag to publish all packages on the GitHub Releases page. Tag builds attach
desktop packages to a draft release while building. After every platform build
succeeds, the final job attaches the APK, AAB and IPA, then automatically
publishes the completed release. A failed build or upload leaves it unpublished.
The Intel and Apple Silicon macOS packages have separate architecture names.

Android release builds require the four Actions secrets above. CI reconstructs
the keystore, signs the APK and AAB, and verifies them before upload. iOS
compilation uses unsigned packages and does not require signing secrets.
CI installs the SDKs and Rust targets, then initializes each native project before
building. The generated projects under `novumchat-backend/gen/` are ignored by this
template; keep any native customization in version control and update the
initialization step if you start maintaining them.
Desktop CI jobs set `TAURI_BUILD_MOBILE=false` because the mobile jobs create
their packages separately on the appropriate runners.

## License

MIT, see [LICENSE](LICENSE). Vendored web assets (KaTeX, highlight.js,
DOMPurify, Mermaid and the Fontsource fonts) keep their own licenses.
