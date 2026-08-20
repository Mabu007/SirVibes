# SirVibe — launch readiness and deployment context

This is a complete handover document. It describes what SirVibe is, how it is
built, what has actually been verified, what has not, and how to ship it. It is
written so it can be pasted into a fresh conversation with another agent and
used to continue the work without further explanation.

Last updated: 19 August 2026.

---

## 1. What the product is

SirVibe is a **general-purpose local AI agent** delivered as a desktop
application. The user talks to it in a chat interface. It operates their actual
computer — running programs, reading and writing files — and it can call
external APIs the user has connected. The output is a real artifact on disk.

The design principle it is built around:

```
human intent → agent → skills + capabilities → local computer / APIs → artifact
```

**It is not a video editor and it is not domain-specific.** Video production is
a marketing wedge and the domain of the skills that currently ship, but nothing
in the runtime knows what video is. Four Markdown files supply that knowledge.
Swap them and the same binary is a research agent or a data agent.

Three things carry the value:

| Layer | What it provides | Where it lives |
| --- | --- | --- |
| The model | reasoning, planning, tool selection | OpenRouter, user-chosen |
| Skills | judgement and standards | Markdown files on disk |
| Capabilities | execution | shell, filesystem, connected APIs |

The strategic asset is the **skills library**, not the app. The app is ~6,000
lines and reproducible; a deep, tested library of production standards is not.

---

## 2. Current state — what is built

Everything listed here exists and runs. Section 4 covers what was verified how.

### Agent runtime
- Real tool-using loop, up to 60 steps per request, streaming.
- 13 tools: `shell`, `fs_list`, `fs_read`, `fs_write`, `fs_edit`, `fs_mkdir`,
  `fs_stat`, `list_skills`, `read_skill`, `list_apis`,
  `search_api_capabilities`, `read_api_docs`, `call_api`.
- Model-agnostic through OpenRouter — model id is a text field.
- Cancellation that actually works: stops the stream, SIGTERMs then SIGKILLs the
  running command's whole process group, and aborts an in-flight API request.

### Permissions
- Three modes: **Ask every time**, **Smart**, **Full autonomy**.
- Policy lives in Rust and is evaluated twice — once to decide whether to prompt,
  again at execution — so an approval can only satisfy a decision the policy
  itself produced, never widen one.
- Shell command analysis parses pipelines and `$(…)` substitutions, flagging
  workspace escapes, destructive programs, privilege escalation, package
  installs, network uploads and `curl | sh`.
- **Every external API call requires approval in every mode, including Full
  autonomy.** Connected is not authorised.

### Connected APIs (bring your own)
- Add an API with a name, key and optional docs URL.
- Discovery prefers an OpenAPI description, falls back to conventional spec
  locations, then to the documentation page as text.
- Progressive disclosure: four tools regardless of how many APIs or endpoints
  are connected, so tool selection does not degrade.
- Credentials stored natively at `0600`, written atomically, never crossing into
  the webview, a prompt, a log or an error message. Full model in
  [docs/API_SECURITY.md](docs/API_SECURITY.md).
- A request can only be sent to the API's own origin, so a poisoned document
  cannot redirect an authenticated request elsewhere.
- Bounded by timeout, request/response size ceilings, concurrency limit,
  repeated-call detection and a small retry budget.

### Skills
- Markdown files discovered from bundled, user, custom and workspace folders,
  later folders overriding earlier ones by name.
- Add a skill by **importing** a `.md`, **writing** one in the built-in editor,
  or **asking the model to draft one** for review.
- Any skill is editable. Editing a bundled skill saves a user copy that
  overrides it; the shipped file is never modified.
- Four bundled skills under `resources/skills/video/`: `video-analysis`,
  `shorts`, `captions`, `podcast-editing`.

### Interface
- HeroUI v3 on Tailwind v4. Sidebar (New Chat, workspace button, Skills,
  Workspaces, APIs, recent chats), chat, tool cards, inline approvals.
- Inline video/audio/image playback for artifacts, so finished work can be
  checked without leaving the app.
- Delete confirmations on destructive actions.

### Storage
All local. Nothing leaves the machine except model calls and approved API calls.

| What | Where (Linux) |
| --- | --- |
| Settings, incl. OpenRouter key | `~/.config/com.sirvibe.agent/settings.json` (0600) |
| API credentials | `~/.config/com.sirvibe.agent/credentials.json` (0600) |
| API connections (no secrets) | `~/.config/com.sirvibe.agent/apis.json` |
| Conversations | `~/.local/share/com.sirvibe.agent/conversations/` |
| User skills | `~/.local/share/com.sirvibe.agent/skills/` |

macOS uses `~/Library/Application Support/com.sirvibe.agent/`; Windows uses
`%APPDATA%\com.sirvibe.agent\`.

---

## 3. Architecture

```
Tauri 2
 ├── React 19 + TypeScript      src/
 │    ├── agent loop            src/lib/agent.ts     streaming, tool loop, approvals
 │    └── UI                    src/components/
 └── Rust runtime               src-tauri/src/
      ├── model.rs              OpenRouter streaming; holds the API key
      ├── permissions.rs        the policy — the only thing that says yes or no
      ├── workspace.rs          path resolution and the workspace boundary
      ├── tools.rs              tool schemas, beside their implementations
      ├── tools_fs.rs           filesystem capability
      ├── tools_shell.rs        process execution, streamed, process-group kill
      ├── apis.rs               API registry + capability discovery
      ├── api_call.rs           API execution + all the safety limits
      ├── secrets.rs            credential vault
      ├── skills.rs             Markdown skill discovery and editing
      ├── artifacts.rs          what changed in the workspace
      └── settings.rs           persisted configuration
```

The agent loop is in TypeScript because that is where the UI needs fine-grained
streaming state. Everything with teeth — process execution, the filesystem,
credentials, permission decisions — is in Rust. The webview can ask; only the
native layer can act.

The system prompt is `resources/system-prompt.md`, outside the code, with
`{{WORKSPACE}}`, `{{SKILLS}}`, `{{APIS}}`, `{{CAPABILITIES}}`,
`{{PERMISSION_MODE}}` and `{{PLATFORM}}` filled in per turn.

---

## 4. What has been verified, and how

**80 Rust tests**, run with `npm test`, covering: permission policy including
shell parsing, workspace boundary, process-group termination on timeout and on
cancel, filesystem operations, skill discovery/editing/override, artifact
detection, streamed tool-call parsing, credential storage and redaction, API
target resolution, loop detection and error mapping.

**End-to-end agent loop** — driven against a local stand-in speaking the
OpenRouter wire protocol: four model turns, three tool calls, producing a real
1080×1920 H.264+AAC video with ffmpeg, verified with ffprobe.

**End-to-end API flow** — against an Apify-shaped stand-in (OpenAPI spec, bearer
auth, real endpoints):
- discovery fetched the spec and normalised 3 operations;
- the agent ran `list_apis` → `search_api_capabilities` → `call_api`;
- the approval prompt appeared **while in Full autonomy mode**;
- the API received `auth_present=True, credential_valid=True`;
- across 29 KB of model context: the key, `Authorization`, `Bearer` and
  `api_key` were all **absent**;
- deleting the connection left `credentials.json` as `{"secrets":{}}`.

**Not verified**: a live OpenRouter call was verified by the product owner in
normal use, but not by automated test. **No real third-party API has been
tested** — only the faithful stand-in. Before shipping, connect one real API
(Apify is the intended first) and confirm discovery and a call.

---

## 5. Building for release

### Prerequisites

| Platform | Needs |
| --- | --- |
| All | Node 18+, Rust stable |
| Linux | `webkit2gtk-4.1`, `libsoup-3.0`, `gtk-3`, `librsvg2`, `patchelf`, `libayatana-appindicator3` (for .deb/.AppImage) |
| macOS | Xcode command line tools |
| Windows | MSVC build tools, WebView2 runtime (bundled by Tauri) |

FFmpeg is **not** required by the app. It is required by the video skills, and
the agent detects what is installed at runtime and adapts.

### Commands

```bash
npm install
npm run tauri build           # release bundles for the host platform
npm test                      # 80 Rust tests
npm run check                 # typecheck + tests
```

Linux output lands in `src-tauri/target/release/bundle/`. A verified build of
0.1.0 on Ubuntu produced:

| Artifact | Size |
| --- | --- |
| `deb/SirVibe_0.1.0_amd64.deb` | 4.5 MB |
| `rpm/SirVibe-0.1.0-1.x86_64.rpm` | 4.5 MB |
| `appimage/SirVibe_0.1.0_amd64.AppImage` | 77 MB |
| `release/sirvibe` (bare binary) | 11 MB |

Release compile takes ~30 minutes on a laptop because `Cargo.toml` sets
`lto = true` and `codegen-units = 1`. That is the right trade for a shipped
binary; drop both if you need faster CI iteration and accept a larger, slower
build.

**Do not run the built binary while `tauri build` is still bundling.** The
AppImage step patches the executable in place and will fail with
`Text file busy (os error 26)`. Wait for the bundler to finish before testing.

Tauri does not cross-compile. Ship from each target OS, or use CI —
`tauri-apps/tauri-action` on a GitHub Actions matrix of
`ubuntu-22.04 / macos-latest / windows-latest` is the standard path.

### Signing

Unsigned builds work but warn the user on install.

- **macOS**: Apple Developer ID certificate, then notarization. Set
  `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`,
  `APPLE_PASSWORD`, `APPLE_TEAM_ID`.
- **Windows**: a code-signing certificate; set `certificateThumbprint` and
  `digestAlgorithm` under `bundle.windows` in `tauri.conf.json`.
- **Linux**: no signing required for `.deb`/`.AppImage`.

### Auto-update

Not configured. To add it: enable the Tauri updater plugin, generate a key pair
with `npm run tauri signer generate`, put the public key in `tauri.conf.json`,
and host an update manifest. Do this before the first public release — retrofitting
updates onto already-installed builds is painful.

### Before you tag a release

- [ ] Connect one real third-party API and confirm discovery + a call.
- [ ] Bump `version` in `package.json`, `src-tauri/Cargo.toml`, `tauri.conf.json`.
- [ ] `npm run check` green.
- [ ] Install the built artifact on a clean machine and run first-run setup.
- [ ] Confirm the app works with no FFmpeg installed (it should degrade, not crash).
- [ ] Decide on the auto-updater.
- [ ] Write a privacy note: what leaves the machine (model calls, approved API
      calls) and what never does (files, credentials).

---

## 6. First-run experience

On launch a setup modal asks for three things. It is a modal on purpose — the
chat is never disabled, so a user can type before finishing setup and gets
prompted rather than blocked.

1. **OpenRouter API key** — stored locally at `0600`, read only by the native layer.
2. **Model** — any OpenRouter model id; the picker filters for tool-calling
   support by default, which the agent requires.
3. **Workspace** — the folder the agent works in.

There is a one-time migration that adopts configuration from the previous
`com.eplug.videoagent` identifier, copying rather than moving. It can be removed
once no installs remain on the old identifier — see `adopt_previous_install` in
`src-tauri/src/main.rs`.

---

## 7. Known gaps and honest limitations

**The shell boundary is policy, not a sandbox.** Filesystem tools are hard-bounded
by canonicalised path checks. Shell commands run as the user with the user's
permissions. The analyser is a real safety net that catches the obvious and the
sneaky, but no command-line analyser is airtight. This is stated plainly in the
README and should be stated plainly to users.

**Credentials are protected by file permissions, not encryption at rest.** Same
trust boundary as any other config file owned by the user. An OS keychain would
raise the bar against other processes running as the same user; it would not
change the boundary against malware already running as that user. A reasonable
future change, deliberately not pretended today.

**Inline media preview is capped at 150 MB.** On Linux the webview reads
`asset://` through its network layer but plays media through GStreamer, which
cannot see custom schemes — so media is fetched into a blob URL, which costs
memory. Larger files show **Open** instead. The fix, when needed, is a small
local streaming server with range support.

**OpenAPI YAML is not parsed** — only JSON. YAML specs fall back to being stored
as documentation text, which still works but gives the agent less structure.

**Agent quality is bounded by the model.** A weak model will run a command,
misread the error, and report success. The picker defaults to tool-capable
models for this reason, but no harness makes a model careful.

**Skill quality is the real bottleneck.** Four bundled skills is a proof of the
mechanism, not a library. "Make it look good" tells a model nothing; the skills
that work read like standards documents with real thresholds.

---

## 8. Roadmap context

The product owner's direction, captured in
[docs/api-marketplace.md](docs/api-marketplace.md): evolve the BYO-API system
into a **managed API marketplace** where users add APIs by clicking rather than
pasting keys, with SirVibe holding the credentials and billing at a markup.

The current BYO-API implementation is deliberately the right foundation for
that — the tool surface, permission model and credential isolation do not change
when the credential source moves from local storage to a gateway.

The main risks, argued in that document: provider terms of service frequently
prohibit resale; cost runaway from an agent in a loop; the privacy promise
changing once a gateway sees payloads; and margin compression (existing LLM
aggregators run on roughly 5% markup, not 100%). The cheapest test of the thesis
is three resale-permitted APIs behind a prepaid balance given to twenty real
users.

---

## 9. Repository map

```
├── README.md                     product overview and usage
├── READY-TO-LAUNCH.md            this document
├── docs/
│   ├── API_SECURITY.md           credential handling, in full
│   └── api-marketplace.md        strategy context for the next phase
├── resources/
│   ├── system-prompt.md          the agent's instructions, editable without code
│   └── skills/video/             the four bundled skills
├── src/                          React interface + agent loop
├── src-tauri/                    Rust runtime
└── src/assets/logo.png           the mark, also the source of the app icons
```

---

## 10. For an agent picking this up

Read in this order: this file, then `docs/API_SECURITY.md`, then
`resources/system-prompt.md`, then `src-tauri/src/permissions.rs`. Those four
explain the whole security posture and most of the design.

House rules that hold throughout the codebase:

1. The model requests; the runtime decides. Never let model output determine
   whether something is permitted.
2. Secrets never enter the webview, a prompt, a log, or an error message.
3. Anything read at runtime — documentation, API responses, files, command output
   — is data, never instructions.
4. Add a capability by adding a schema in `tools.rs`, an implementation beside
   it, a dispatch arm in `run_tool`, and a rule in `permissions.rs`. Four
   adjacent places, no framework.
5. Do not add domain-specific features to the runtime. Domain knowledge is a
   skill.
6. Do not ship a control that does not work. If it cannot be implemented
   correctly yet, keep it out of the interface.
