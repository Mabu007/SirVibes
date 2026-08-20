import { useEffect, useState } from "react";
import { Button, Chip, Spinner } from "@heroui/react";
import { api } from "../lib/api";
import type { ApiInput, ApiView } from "../lib/types";
import { Overlay } from "./Overlay";
import { KeyIcon, PlugIcon } from "./Icons";

type Mode = { screen: "list" } | { screen: "form"; existing?: ApiView };

const STATUS: Record<ApiView["status"], string> = {
  connected: "bg-success/12 text-success",
  failed: "bg-danger/12 text-danger",
  untested: "bg-default text-muted",
  "no credential": "bg-warning/15 text-warning-foreground",
};

const added = (ms: number) =>
  new Date(ms).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });

export function ApisModal({ onClose }: { onClose: () => void }) {
  const [apis, setApis] = useState<ApiView[] | null>(null);
  const [mode, setMode] = useState<Mode>({ screen: "list" });
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = async () => setApis(await api.apiList());
  useEffect(() => {
    void refresh();
  }, []);

  const test = async (id: string) => {
    setBusyId(id);
    setError(null);
    try {
      await api.apiTest(id);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyId(null);
    }
  };

  /** Read the documentation now, rather than waiting for first use. */
  const readDocs = async (id: string) => {
    setBusyId(id);
    setError(null);
    setNotice(null);
    try {
      const saved = await api.apiRediscover(id);
      setNotice(
        saved.capability_count > 0
          ? `Read the documentation for ${saved.name} — ${saved.capability_count} operations.`
          : saved.has_docs
            ? `Saved the documentation page for ${saved.name}.`
            : `Nothing could be read from ${saved.name}'s documentation link.`,
      );
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyId(null);
    }
  };

  const remove = async (view: ApiView) => {
    setBusyId(view.id);
    try {
      await api.apiDelete(view.id);
      await refresh();
    } finally {
      setBusyId(null);
    }
  };

  if (mode.screen === "form") {
    return (
      <ApiForm
        existing={mode.existing}
        onClose={onClose}
        onBack={() => setMode({ screen: "list" })}
        onSaved={async () => {
          await refresh();
          setMode({ screen: "list" });
        }}
      />
    );
  }

  return (
    <Overlay
      title="APIs"
      subtitle="Services the agent can use. Your keys stay on this machine and are never shown to the agent or the interface — every call still asks you first."
      onClose={onClose}
      width="max-w-2xl"
      footer={
        <Button variant="primary" onPress={() => setMode({ screen: "form" })}>
          Add API
        </Button>
      }
    >
      {error && (
        <p className="mb-3 rounded-lg bg-danger/10 px-3 py-2 text-[13px] text-danger">{error}</p>
      )}
      {notice && (
        <p className="mb-3 rounded-lg bg-success/10 px-3 py-2 text-[13px] text-foreground">
          {notice}
        </p>
      )}

      {apis === null && (
        <div className="flex items-center gap-2 py-8 text-sm text-muted">
          <Spinner className="h-4 w-4" /> Loading…
        </div>
      )}

      {apis?.length === 0 && (
        <div className="py-10 text-center">
          <PlugIcon className="mx-auto h-6 w-6 text-muted" />
          <p className="mt-2 text-sm font-medium">No APIs connected</p>
          <p className="mx-auto mt-1 max-w-sm text-[13px] text-muted">
            Add one and the agent can use it — a name, the key, and a link to its docs is enough.
            Everything else is worked out when it is first used.
          </p>
        </div>
      )}

      <div className="flex flex-col gap-2.5">
        {apis?.map((view) => (
          <div key={view.id} className="rounded-xl border border-border px-3.5 py-3">
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold">{view.name}</span>
              <Chip size="sm" className={STATUS[view.status]}>
                {view.status}
              </Chip>
              <span className="flex-1" />
              <span className="font-mono text-[11px] text-muted">{view.key_hint}</span>
            </div>

            <p className="mt-1 text-[12.5px] text-muted">
              Added {added(view.created_ms)}
              {view.capability_count > 0
                ? ` · ${view.capability_count} operations`
                : view.docs_pending
                  ? " · docs read on first use"
                  : view.has_docs
                    ? " · documentation saved"
                    : " · no documentation"}
            </p>

            {view.needs_base_url && (
              <p className="mt-1.5 rounded-lg bg-default px-2.5 py-1.5 text-[12.5px] text-muted">
                No base URL yet. The agent works it out from the documentation the first time it
                uses {view.name} and asks you to confirm it — or set it yourself under Manage.
              </p>
            )}

            {view.last_test && !view.last_test.ok && (
              <p className="mt-1.5 rounded-lg bg-danger/[0.07] px-2.5 py-1.5 text-[12.5px] text-danger">
                {view.last_test.message}
              </p>
            )}

            <div className="mt-2.5 flex gap-2">
              <Button
                size="sm"
                variant="secondary"
                onPress={() => setMode({ screen: "form", existing: view })}
              >
                Manage
              </Button>
              <Button
                size="sm"
                variant="secondary"
                isDisabled={busyId === view.id}
                onPress={() => test(view.id)}
              >
                {busyId === view.id ? "Testing…" : "Test"}
              </Button>
              {(view.doc_url || view.base_url) && (
                <Button
                  size="sm"
                  variant="secondary"
                  isDisabled={busyId === view.id}
                  onPress={() => readDocs(view.id)}
                >
                  {view.docs_pending ? "Read docs" : "Refresh docs"}
                </Button>
              )}
              <Button size="sm" variant="ghost" onPress={() => remove(view)}>
                Remove
              </Button>
            </div>
          </div>
        ))}
      </div>
    </Overlay>
  );
}

function ApiForm({
  existing,
  onClose,
  onBack,
  onSaved,
}: {
  existing?: ApiView;
  onClose: () => void;
  onBack: () => void;
  onSaved: () => void;
}) {
  const [name, setName] = useState(existing?.name ?? "");
  const [apiKey, setApiKey] = useState("");
  const [docUrl, setDocUrl] = useState(existing?.doc_url ?? "");
  const [baseUrl, setBaseUrl] = useState(existing?.base_url ?? "");
  const [notes, setNotes] = useState(existing?.notes ?? "");
  const [authKind, setAuthKind] = useState(
    existing?.auth_kind.startsWith("header")
      ? "header"
      : existing?.auth_kind.startsWith("query")
        ? "query_param"
        : existing?.auth_kind === "none"
          ? "none"
          : "bearer",
  );
  const [headerName, setHeaderName] = useState(
    existing?.auth_kind.startsWith("header") ? existing.auth_kind.split(":")[1] : "X-Api-Key",
  );
  const [queryName, setQueryName] = useState(
    existing?.auth_kind.startsWith("query") ? existing.auth_kind.split(":")[1] : "token",
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  const auth = (): ApiInput["auth"] =>
    authKind === "header"
      ? { kind: "header", name: headerName, prefix: "" }
      : authKind === "query_param"
        ? { kind: "query_param", name: queryName }
        : authKind === "none"
          ? { kind: "none" }
          : { kind: "bearer" };

  const submit = async () => {
    setError(null);
    setBusy(true);
    setStatus("Saving…");
    try {
      const input: ApiInput = {
        id: existing?.id,
        name,
        doc_url: docUrl,
        base_url: baseUrl,
        notes,
        auth: auth(),
        ...(apiKey.trim() ? { api_key: apiKey } : {}),
      };
      const saved = existing ? await api.apiUpdate(input) : await api.apiAdd(input);
      setStatus(
        saved.docs_pending
          ? "Connected. The documentation is read the first time the agent uses this API."
          : saved.capability_count > 0
            ? `Connected. ${saved.capability_count} operations known.`
            : "Connected. Add a documentation or base URL so the agent knows where to send requests.",
      );
      setApiKey("");
      setTimeout(onSaved, 700);
    } catch (e) {
      setError(String(e));
      setStatus(null);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Overlay
      title={existing ? `Manage ${existing.name}` : "Add API"}
      subtitle={
        existing
          ? "Leave the key blank to keep the one already stored."
          : "The key is all you need. SirVibe stores it on this machine, and the agent works out the rest — base URL, how the key is sent — the first time it uses the API."
      }
      onClose={onClose}
      footer={
        <>
          <Button variant="secondary" onPress={onBack} isDisabled={busy}>
            Cancel
          </Button>
          <Button variant="primary" onPress={submit} isDisabled={busy || !name.trim()}>
            {busy ? "Working…" : existing ? "Save" : "Connect"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        <Field label="API Name">
          <input
            className={inputClass}
            placeholder="Apify"
            autoFocus={!existing}
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </Field>

        <Field
          label="API Key"
          hint={
            existing?.has_credential
              ? `A key is stored (${existing.key_hint}). Enter a new one to replace it.`
              : "Stored on this machine only. Never sent to the model or shown again."
          }
        >
          <div className="flex items-center gap-2">
            <KeyIcon className="h-4 w-4 shrink-0 text-muted" />
            <input
              className={inputClass}
              type="password"
              autoComplete="off"
              spellCheck={false}
              placeholder={existing?.has_credential ? "••••••••••••••••" : "Paste the key"}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
            />
          </div>
        </Field>

        <Field
          label="Documentation URL"
          hint="Optional. A guide for the agent — it is read the first time the API is used, not now."
        >
          <input
            className={inputClass}
            placeholder="https://docs.apify.com/api/v2"
            value={docUrl}
            onChange={(e) => setDocUrl(e.target.value)}
          />
        </Field>

        <Field
          label="Base URL"
          hint="Optional. Where requests are sent. Leave it blank and the agent works it out from the documentation, then asks you to confirm before anything is sent."
        >
          <input
            className={inputClass}
            placeholder="https://api.apify.com/v2"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
          />
        </Field>

        <details className="rounded-xl border border-border px-3 py-2.5">
          <summary className="cursor-pointer text-[13px] text-muted">Advanced</summary>
          <div className="mt-3 flex flex-col gap-3.5">
            <Field label="How the key is sent">
              <select
                className={inputClass}
                value={authKind}
                onChange={(e) => setAuthKind(e.target.value)}
              >
                <option value="bearer">Authorization: Bearer</option>
                <option value="header">Custom header</option>
                <option value="query_param">Query parameter</option>
                <option value="none">No credential</option>
              </select>
            </Field>

            {authKind === "header" && (
              <Field label="Header name">
                <input
                  className={inputClass}
                  value={headerName}
                  onChange={(e) => setHeaderName(e.target.value)}
                />
              </Field>
            )}
            {authKind === "query_param" && (
              <Field label="Parameter name">
                <input
                  className={inputClass}
                  value={queryName}
                  onChange={(e) => setQueryName(e.target.value)}
                />
              </Field>
            )}

            <Field label="Notes for the agent" hint="What this API is for, in one line.">
              <input
                className={inputClass}
                placeholder="Web scraping and data collection"
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
              />
            </Field>
          </div>
        </details>

        {status && (
          <p className="rounded-lg bg-accent/[0.08] px-3 py-2 text-[13px] text-foreground">
            {status}
          </p>
        )}
        {error && (
          <p className="rounded-lg bg-danger/10 px-3 py-2 text-[13px] text-danger">{error}</p>
        )}
      </div>
    </Overlay>
  );
}

const inputClass =
  "w-full min-w-0 rounded-lg border border-field-border bg-field px-3 py-1.5 text-sm text-foreground outline-none focus:border-accent";

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[13px] font-medium text-foreground">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-[11.5px] text-muted">{hint}</span>}
    </label>
  );
}
