# Connected apps (Composio)

How SirVibe connects external applications — Gmail, GitHub, Google Drive — so a
user signs in instead of pasting an app's API key, and how an agent reaches
those apps afterwards. This describes the code as built; if you change the
integration, change this document in the same commit.

---

## 1. What Composio is doing here

Composio brokers OAuth to third-party applications and holds the resulting
tokens. SirVibe never receives an app's access token, refresh token, or
password. It holds one credential — the Composio **project API key** — and
Composio applies the app's own credential server-side when an action runs.

```
User → SirVibe → agent → Composio → connected app → app actions
```

The whole integration is REST over `https://backend.composio.dev/api/v3`,
written against Composio's published OpenAPI document. **No SDK is vendored and
no dependency was added**; it reuses the `reqwest` client the project already
had.

## 2. Credentials

| Credential | Where it lives | Who can read it |
|---|---|---|
| Composio project API key | `SecretStore` under id `composio`, or `COMPOSIO_API_KEY` | the native process only |
| Connected app's OAuth tokens | Composio's servers | nobody in this process |

The project key follows the existing rules in `API_SECURITY.md` exactly:

- stored in `<app config dir>/credentials.json`, mode `0600`;
- read at the moment a request is signed, never held in a struct;
- sent only as an `x-api-key` header — never in a URL, body, or log;
- every Composio response is passed through `secrets::redact` before anything
  else sees it;
- the frontend is told only `configured`, a masked `key_hint`, and whether the
  key came from the environment. `apps_status` is the only command that speaks
  about it, and it cannot return the key.

`COMPOSIO_API_KEY` is a fallback for machines that provision the key outside the
UI. A key saved through the panel takes precedence.

## 3. User scoping

Composio partitions connections by a `user_id` supplied on every call. SirVibe
is a single-user local desktop application with no account system, so it
generates one stable local identifier — `settings.composio_user_id`, of the form
`sirvibe-<hex>` — on first use and persists it.

Two rules keep connections apart:

1. `AppRegistry` is keyed on `(user_id, toolkit_slug)`. `for_user` is the only
   listing the rest of the application uses, so a connection cannot be read
   across users by forgetting a filter.
2. Every Composio call that touches a connection passes that `user_id`.

**Limitation worth knowing.** This identifier names *this install*, not a
person. It is not an authentication boundary: anyone with access to the machine
account is that user. If SirVibe ever grows real accounts, replace the single
`composio_user_id` with the signed-in user's id — `AppRegistry` and every
Composio call already take a `user_id` and need no other change.

## 4. The connection flow

`apps_connect(toolkit_slug)`:

1. `GET /toolkits/{slug}` — confirm the app exists and that Composio can broker
   its sign-in (`no_auth`, or a non-empty `composio_managed_auth_schemes`).
2. `GET /auth_configs?toolkit_slug=` — reuse the project's existing registration
   if there is one, so repeated connects do not pile up duplicates.
3. `POST /auth_configs` with `use_composio_managed_auth` if there is not.
4. `POST /connected_accounts/link` with `{auth_config_id, user_id}` → a
   `redirect_url` and a `connected_account_id`.
5. The redirect URL is opened in the user's **real browser** via
   `tauri-plugin-opener`, where their existing sessions and password manager
   are. It is never rendered in a window SirVibe controls.
6. The row is recorded locally with status `INITIATED`.

Composio hosts the OAuth callback, so SirVibe runs no local web server and
registers no redirect URI of its own. `apps_check` polls
`GET /connected_accounts/{id}` until the status reaches `ACTIVE` or a terminal
failure.

> `POST /connected_accounts` is **not** used. Composio is retiring it for
> managed-auth OAuth configs; `/connected_accounts/link` is the current path.

## 5. How an agent uses a connected app

Three tools, regardless of how many apps are connected or how many thousands of
actions they expose — the same progressive disclosure the API tools use:

1. `list_connected_apps` — answered from the local record; instant, works
   offline.
2. `search_app_tools` — `GET /tools?toolkit_slug=&search=` at runtime, capped at
   10 results, **scoped to what this user has actually connected**. Only the
   matched tools' schemas enter the model's context.
3. `run_app_tool` — `POST /tools/execute/{slug}` with the user's `user_id` and
   `connected_account_id`.

No tool catalogue is ever preloaded into a prompt. The system prompt lists only
connected app names and their `app_id`.

Before executing, `run_app_tool` re-reads the connection from Composio. A token
revoked five minutes ago fails here rather than producing a confusing error from
the app.

## 6. Permission model

`run_app_tool` is `Decision::Ask` in **every** permission mode, including Full
autonomy. Workspace autonomy is autonomy over the workspace, and someone's inbox
is not in it. `list_connected_apps` and `search_app_tools` read catalogues and
are `Allow`.

The approval prompt names the app, the action, the purpose, and the **argument
names** being sent — names only, because values can carry the user's own words.

`app_call_info` builds that description from the local registry with no network
call, using Composio's `<TOOLKIT>_<ACTION>` slug convention. That is a
description, not the security boundary: execution independently resolves the
tool's real toolkit through Composio and re-checks the connection.

## 7. Failure handling

Every one of these is a distinct, actionable message, never a swallowed error:

| Condition | Reported as |
|---|---|
| No `COMPOSIO_API_KEY` | Apps unavailable; where to add one |
| Key rejected (401) | Check the key in the Apps panel |
| Project not permitted (403) | Check permissions in the Composio dashboard |
| Toolkit needs custom OAuth | Named, with what to do about it |
| OAuth not finished / cancelled | Distinguished from failure |
| Link expired | Suggests reconnecting |
| `EXPIRED` / `REVOKED` / `FAILED` / `INACTIVE` | Each explained separately |
| Disconnected outside SirVibe | Stale row dropped on refresh |
| Tool returns `successful: false` | Surfaced as an error, not a silent no-op |
| Rate limited (429) | Wait and retry |
| Composio 5xx / timeout / offline | Named as Composio's side vs the network |

`apps_disconnect` drops the local row even when the remote revoke fails, so a
dead connection cannot get stuck in the list — but it still reports that the
revoke did not happen.

## 8. Files

| File | Role |
|---|---|
| `src-tauri/src/composio.rs` | REST client, models, error mapping |
| `src-tauri/src/apps.rs` | User-scoped local registry (no credentials) |
| `src-tauri/src/main.rs` | `apps_*` commands, `run_apps_tool`, `app_call_info` |
| `src-tauri/src/tools.rs` | The three agent tool schemas |
| `src-tauri/src/permissions.rs` | `AppTarget`, `evaluate_app_tool` |
| `src/components/AppsModal.tsx` | Apps panel |

Adding another app requires **no** SirVibe code: it is a search and a Connect
button, resolved entirely through Composio's catalogue.
