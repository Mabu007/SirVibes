# SirVibe

A general-purpose desktop AI agent. You talk to it; it operates your computer and
connects to your APIs to produce real results. Video production is one domain it
is good at, supplied through skills — not something baked into the runtime.

```
human intent → agent → skills + capabilities → local computer → artifact
```

## What it is

A small Tauri app wrapped around a real tool-using agent loop. The model reasons
and plans, Markdown *skills* supply the judgement, and generic *capabilities* do
the work:

- **Local** — a shell and a filesystem. FFmpeg, Python and anything else
  installed on your machine are reached through the shell.
- **External** — APIs you connect yourself. You add a name, a key and a docs
  link; SirVibe stores the credential natively, works out what the API can do,
  and lets the agent use it. Every call asks you first.

The runtime contains no domain-specific code at all.

## Bring your own APIs

Add an API in the sidebar under **APIs**: a name, its key, and optionally a link
to its documentation. SirVibe fetches the docs, prefers an OpenAPI description
when there is one, and turns it into operations the agent can search.

The agent never sees your key. It asks for `call_api(api_id: "apify", …)` and the
native layer resolves the credential while signing the request. Full details in
[docs/API_SECURITY.md](docs/API_SECURITY.md).

Rather than registering every endpoint as a model tool — which wrecks tool
selection once you have a few APIs — the agent gets four: `list_apis`,
`search_api_capabilities`, `read_api_docs` and `call_api`. A hundred connected
APIs with five thousand endpoints is still four tools.

**Every external API call requires your approval, in every permission mode,
including Full autonomy.** Connected is not authorised. Alongside that, each call
is bounded by a timeout, request and response size ceilings, a concurrency limit,
a repeated-call detector and a small retry budget — those stop runaway loops;
your approval is the spending control.

## Running it

Requires Node 18+, a Rust toolchain, and the usual Tauri Linux deps
(`webkit2gtk-4.1`, `libsoup-3.0`, `gtk3`). FFmpeg is not required by the app but
the agent will want it. The interface is built with [HeroUI](https://heroui.com)
v3 on Tailwind CSS v4.

```bash
npm install
npm run tauri dev      # development
npm run tauri build    # bundle
```

On first launch a setup modal asks for three things. It is a modal on purpose —
the chat is never disabled, so you can type before you have finished setting up
and the app will prompt you rather than swallowing what you wrote. Everything is
also editable later in **Settings**:

1. **OpenRouter API key** — stored in the app config file with `0600`
   permissions and only ever read by the Rust layer.
2. **Model** — any OpenRouter model id. The picker lists live models and
   defaults to filtering for ones that support tool calling, which the agent
   needs.
3. **Workspace** — the folder the agent works in.
4. **Permission mode** — see below.

## Permission modes

The model *requests* an action; the runtime decides whether it happens. Policy
lives in Rust (`src-tauri/src/permissions.rs`) and is evaluated twice: once to
decide whether to show you a prompt, and again at execution time, so an approval
can only satisfy a decision the policy itself produced — never widen one.

| Mode | Behaviour |
| --- | --- |
| **Ask every time** | Every tool call waits for approval. |
| **Smart** | Routine production work runs. Deletions, package installs, privilege escalation, network uploads, `curl \| sh`, and anything outside the workspace ask first. |
| **Full autonomy** | Everything inside the workspace runs unattended, including destructive commands. Leaving the workspace still asks. |

`list_skills` and `read_skill` are exempt in every mode: they only read the
agent's own instruction files and never touch your data, so prompting for them
would be noise.

**Stop** is real. It cancels the model stream, and sends `SIGTERM` (then
`SIGKILL`) to the process group of whatever command is running, so a ten-minute
render actually stops. Commands lead their own process group, which is also how
the per-command timeout reaches a tool that `sh` forked rather than exec'd.

### What the sandbox actually guarantees

Be clear-eyed about this:

- **Filesystem tools are hard-bounded.** Paths are resolved and canonicalized
  against the workspace root, symlinks included, so `fs_*` cannot silently
  escape it.
- **Shell commands are bounded by policy, not by a sandbox.** Commands run as
  your user with your permissions, from the workspace directory. The analyzer in
  `permissions.rs` parses the command line — including pipelines and `$(…)`
  substitutions — and flags escapes, destructive programs, installs and uploads,
  but a sufficiently creative command can evade any such analysis. It is a
  meaningful safety net, not a jail. If you need a true boundary, run the app
  against a workspace on a machine or container you are willing to lose.

## Tests

```bash
npm test          # 46 Rust tests: policy, sandbox, shell, skills, artifacts, stream parsing
npm run check     # the above plus a TypeScript typecheck
```

The permission analyzer, the workspace boundary, process-group termination on
timeout, skill discovery, artifact detection and the streamed tool-call parser
all have tests. In a **debug build only**, `SIRVIBE_MODEL_BASE_URL` points the
model client at a local OpenAI-compatible server, which is how the full agent
loop is exercised without spending tokens. The override is compiled out of
release builds, so it can never redirect a real API key.

## Playing artifacts in the app

Video, audio and image artifacts play inline in the conversation, so finished
work can be checked without leaving the app. Files are served straight from the
workspace over Tauri's asset protocol, whose scope is granted for the active
project folder and nothing else.

One platform wrinkle worth knowing: on Linux the webview reads `asset://`
through its network layer but plays media through GStreamer, which cannot see
custom schemes — a `<video src="asset://…">` fails with `SRC_NOT_SUPPORTED`
even though the file is served correctly. Media is therefore fetched once into
a blob URL, which costs memory, so anything above 150 MB shows **Open** instead
of an inline player.

## Skills

A skill is a Markdown file. That is the whole mechanism.

```
skills/
  shorts.md
  captions.md
  my-house-style.md
```

Skills are discovered from, in order: the bundled `resources/skills`, your user
skills folder (Settings → *Open skills folder*), any folders you add, and
`<workspace>/skills`. Later ones win, so you can override a bundled skill by
name.

Optional frontmatter gives the agent a better index:

```markdown
---
name: my-house-style
description: How our channel's videos are cut and graded.
when_to_use: Any deliverable going out on the main channel.
---

# My House Style
...
```

The six bundled skills (`video-analysis`, `shorts`, `captions`, `hyperframes`,
`music`, `podcast-editing`) are loaded by the same loader from the same format
as yours. There is no hidden implementation behind any of them — the
captions skill is editorial knowledge, and the mechanics it points at are the
HyperFrames CLI and ffmpeg, both driven through the shell like anything else.

## Architecture

```
Tauri
 ├── React UI            src/            HeroUI: sidebar, chat, tools, players
 │    └── Agent loop     src/lib/agent.ts
 └── Rust runtime        src-tauri/src/
      ├── model.rs       OpenRouter (streaming, tool calls); holds the API key
      ├── permissions.rs the policy — the only thing that says yes or no
      ├── workspace.rs   path resolution and the workspace boundary
      ├── tools.rs       tool schemas, next to their implementations
      ├── tools_fs.rs    filesystem capability
      ├── tools_shell.rs process execution, streamed live
      ├── skills.rs      Markdown skill discovery
      └── artifacts.rs   what changed in the workspace
```

The agent loop runs in TypeScript because that is where the UI needs
fine-grained streaming state. Everything with teeth — process execution, the
filesystem, the API key, permission decisions — is in Rust.

The system prompt lives in `resources/system-prompt.md`, outside the code, with
`{{WORKSPACE}}`, `{{SKILLS}}`, `{{CAPABILITIES}}`, `{{PERMISSION_MODE}}` and
`{{PLATFORM}}` filled in per turn. Edit it without touching a line of Rust.

## Adding a capability

Add the schema to `tools.rs`, the implementation next to it, a case in
`run_tool` in `main.rs`, and a rule in `permissions.rs`. Four places, all
adjacent, no framework.

## Storage

Everything is local. Settings in the app config dir, conversations as JSON in
the app data dir, artifacts in your workspace. No database, no accounts, no
cloud — the only network traffic is to OpenRouter.
