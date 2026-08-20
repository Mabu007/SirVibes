# API credential security

How SirVibe stores API keys, who can read them, how they reach a request, and
what deliberately never happens to them. This describes the code as built — if
you change the credential path, change this document in the same commit.

---

## 1. Where credentials are stored

One file, in the application's own config directory:

```
<app config dir>/credentials.json
```

On Linux that is `~/.config/com.sirvibe.agent/credentials.json`.

It holds a flat map of connection id to plaintext key:

```json
{ "secrets": { "apify": "apify_api_…" } }
```

Nothing else lives in that file. Connection metadata — name, base URL,
documentation URL, discovered operations — is stored **separately** in
`apis.json`, which contains no secrets. The split is deliberate: anything that
reads, exports, or logs connection records cannot touch a credential by
accident.

Implementation: `src-tauri/src/secrets.rs`.

## 2. How it is protected at rest

- The file is written with mode `0600` — readable and writable by the owning
  user account only. Every write re-applies the mode.
- Writes go to a temporary file which is then renamed over the target, so an
  update replaces the previous secret **atomically**. A crash mid-write cannot
  leave a half-written vault or a mixture of old and new keys.

**Be clear about what this is not.** The file is not encrypted at rest. It is
protected by filesystem permissions, exactly like the OpenRouter key in
`settings.json`. Anything running as your user account can read it — that is
the same trust boundary the agent itself already operates within, since it can
run shell commands as you. Moving to an OS keychain would raise the bar against
other processes running as the same user; it would not change the boundary
against malware that already has your account. It is a reasonable future change,
not a pretence made today.

## 3. Which process can read them

Only the native Rust process. `SecretStore::get` is the single function that
returns plaintext, and it is called from exactly one place:
`api_call::execute`, while a request is being signed.

There is **no Tauri command that returns a credential**. The webview cannot ask
for one, because no such command exists to call.

## 4. The path from storage to a request

```
Model                     "call apify, actor = X"      (no credential)
  │
  ▼
Permission layer          user approves this call      (no credential)
  │
  ▼
Native call executor      SecretStore::get("apify")    ← plaintext read here
  │
  ▼
HTTP request              Authorization: Bearer …      ← plaintext used here
  │
  ▼
External API
```

The plaintext exists only inside `execute`, only after approval, and only for
the lifetime of that request. It is placed into the request by one of the
supported schemes — bearer token, a named header, or a query parameter —
according to the connection's `auth` setting.

## 5. Why credentials never enter the webview

The interface is a browser context. Anything it holds can appear in memory
snapshots, devtools, crash reports, and any bug that serialises component state.
So the interface receives an `ApiView`, which carries a **masked hint**
(`••••••••1234`) and a `has_credential` boolean, and nothing more.

The hint is derived from the last four characters. It exists so a person can
tell two keys apart. It cannot be used to authenticate.

There is a test asserting that a serialised `ApiView` does not contain the
stored secret. If someone adds a field that leaks one, that test fails.

## 6. Why credentials never enter a prompt

The model is told an API exists, what it is for, and what operations it has. It
is never told the key, and it never needs to be: it asks for
`call_api(api_id: "apify", …)` and the native layer resolves the credential.

This matters beyond tidiness. Anything placed in a prompt is: sent to a third
party (the model provider), retained in conversation history on disk, and
exposed to prompt-injection attacks that try to make the model repeat its
context. A credential in a prompt is a credential published.

The same reasoning bars credentials from **skills**. A skill may name an API and
a capability; it must never carry a key.

## 7. Why credentials never enter logs or errors

Upstream APIs sometimes echo an `Authorization` header back in an error body.
So there are two defences:

1. Nothing that carries a credential is ever passed to a log or a result. The
   request builder is the only holder.
2. Every response body and every error message is passed through
   `secrets::redact` before it reaches the model, the interface, or a log,
   which strips the known secret and anything shaped like an auth header.

Error messages are written for people and name the API, not the credential:

> API authentication failed.
> The API key was rejected by Apify. Check the key in the API manager and try again.

Safe to record: `api_id`, operation, timestamp, duration, status,
response size, byte counts. Not recorded by default: request and response
payloads.

## 8. Deletion

Removing a connection calls `SecretStore::remove`, which rewrites the vault
without that entry, then removes the connection record. Both must succeed for
the removal to report success.

After deletion the key is gone from disk, the connection disappears from
`list_apis`, and `call_api` for that id fails with "not connected" — so the
capability is genuinely unavailable, not merely hidden.

A test asserts the removed secret is absent from the file's bytes afterwards.

## 9. How requests are authenticated

Per connection, one of:

| Scheme | Sent as |
| --- | --- |
| `bearer` (default) | `Authorization: Bearer <key>` |
| `header` | a named header, optionally with a prefix |
| `query_param` | a named query string parameter |
| `none` | nothing — for APIs that need no credential |

**A request can only be sent to the API's own origin.** If a path resolves to a
different scheme or host than the connection's base URL, the call is refused
before any credential is read. This is what stops a poisoned documentation page
or a manipulated model from pointing an authenticated request at somebody
else's server. There is a test for it.

## 10. How permissions protect API calls

Every external API call requires explicit user approval, **in every permission
mode**, including Full autonomy. Connected is not authorised: adding an API
grants the agent the ability to propose a call, never to make one.

The approval prompt is built natively from the stored connection and the
resolved target — not from model output — so a request cannot be described to
the user as something other than what it is. It shows the API, the operation,
the method and URL, the agent's stated purpose, the parameters, and whether the
call can change remote state.

Approval is asked for immediately before execution, and it applies to that one
call. There is no "always allow"; that is a deliberate omission for V1.

Alongside approval, every call is bounded: a timeout, a request-size ceiling, a
response-size ceiling, a concurrency limit, a repeated-call detector, and a
small retry budget for transient failures only. These stop runaway loops. They
are not the spending control — the user approving each call is.

## 11. Untrusted input

Documentation pages, API responses, web pages, files and command output are
**data**. They cannot grant permission, change the agent's instructions, or
authorise a call. Retrieved HTML has `<script>` and `<style>` stripped before
storage, documentation handed to the model is labelled as third-party text, and
the system prompt instructs the agent to report rather than obey any content
that tries to give it orders.

The permission layer is the authority. Nothing read at runtime can move it.

## 12. For future capability providers

MCP servers, additional network providers and process-backed capabilities must
follow the same model:

1. Secrets in `SecretStore`, keyed by provider id — never in the provider's own
   config record.
2. No command returns a secret to the webview. Views carry masked hints.
3. The provider id travels through the model; the credential is resolved
   natively at call time.
4. Every outbound call goes through the permission layer and is presented to the
   user before it runs.
5. Destinations are constrained to the provider's declared origin.
6. Responses are redacted and size-capped before they reach the model.

If a new provider type cannot meet all six, it is not ready to ship.
