import { useEffect, useRef, useState } from "react";
import { Button, Chip, Spinner } from "@heroui/react";
import { api } from "../lib/api";
import type { AppView, AppsStatus, Toolkit } from "../lib/types";
import { Overlay } from "./Overlay";
import { AppsIcon, KeyIcon, SearchIcon } from "./Icons";

type Screen =
  | { name: "list" }
  | { name: "browse" }
  | { name: "key" }
  | { name: "waiting"; app: string; label: string; url: string };

/** How long to keep asking whether a sign-in finished before giving up on it. */
const POLL_MS = 2500;
const POLL_LIMIT = 96; // four minutes

const inputClass =
  "w-full min-w-0 rounded-lg border border-field-border bg-field px-3 py-1.5 text-sm text-foreground outline-none focus:border-accent";

export function AppsModal({ onClose }: { onClose: () => void }) {
  const [status, setStatus] = useState<AppsStatus | null>(null);
  const [apps, setApps] = useState<AppView[] | null>(null);
  const [screen, setScreen] = useState<Screen>({ name: "list" });
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busySlug, setBusySlug] = useState<string | null>(null);

  // Load what is connected from the local record first — that is instant — then
  // reconcile against Composio so a revoked connection stops looking healthy.
  useEffect(() => {
    void (async () => {
      const s = await api.appsStatus();
      setStatus(s);
      setApps(await api.appsList());
      if (!s.configured) {
        setScreen({ name: "key" });
        return;
      }
      try {
        setApps(await api.appsRefresh());
      } catch (e) {
        setError(`Could not check your connections with Composio. ${String(e)}`);
      }
    })();
  }, []);

  const reload = async () => setApps(await api.appsList());

  const disconnect = async (app: AppView) => {
    setBusySlug(app.toolkit_slug);
    setError(null);
    setNotice(null);
    try {
      await api.appsDisconnect(app.toolkit_slug);
      setNotice(`${app.name} disconnected.`);
    } catch (e) {
      // The local record is dropped even when the revoke fails, so say what
      // actually happened rather than pretending it went cleanly.
      setError(String(e));
    } finally {
      setBusySlug(null);
      await reload();
    }
  };

  if (screen.name === "key") {
    return (
      <KeyScreen
        status={status}
        onClose={onClose}
        onBack={status?.configured ? () => setScreen({ name: "list" }) : undefined}
        onSaved={async (s) => {
          setStatus(s);
          setScreen({ name: "list" });
          try {
            setApps(await api.appsRefresh());
          } catch {
            await reload();
          }
        }}
      />
    );
  }

  if (screen.name === "browse") {
    return (
      <BrowseScreen
        connected={apps ?? []}
        onClose={onClose}
        onBack={() => setScreen({ name: "list" })}
        onStarted={async (slug, label, url) => {
          await reload();
          setScreen({ name: "waiting", app: slug, label, url });
        }}
      />
    );
  }

  if (screen.name === "waiting") {
    return (
      <WaitingScreen
        slug={screen.app}
        label={screen.label}
        url={screen.url}
        onClose={onClose}
        onDone={async (message, failed) => {
          if (failed) setError(message);
          else setNotice(message);
          await reload();
          setScreen({ name: "list" });
        }}
      />
    );
  }

  return (
    <Overlay
      title="Apps"
      subtitle="Applications the agent can act on, connected by signing in — not by pasting an API key. Every action still asks you first."
      onClose={onClose}
      width="max-w-2xl"
      footer={
        <>
          <Button variant="secondary" onPress={() => setScreen({ name: "key" })}>
            Composio key
          </Button>
          <Button variant="primary" onPress={() => setScreen({ name: "browse" })}>
            Add App
          </Button>
        </>
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

      {apps === null && (
        <div className="flex items-center gap-2 py-8 text-sm text-muted">
          <Spinner className="h-4 w-4" /> Loading…
        </div>
      )}

      {apps?.length === 0 && (
        <div className="py-10 text-center">
          <AppsIcon className="mx-auto h-6 w-6 text-muted" />
          <p className="mt-2 text-sm font-medium">No apps connected</p>
          <p className="mx-auto mt-1 max-w-sm text-[13px] text-muted">
            Add one and the agent can work in it on your behalf — read your mail, file something in
            Drive, open an issue. You sign in once; SirVibe never sees the password or the token.
          </p>
        </div>
      )}

      <div className="flex flex-col gap-2">
        {apps?.map((app) => (
          <div
            key={app.toolkit_slug}
            className="flex items-center gap-3 rounded-xl border border-border px-3.5 py-3"
          >
            <AppMark name={app.name} logo={app.logo} />
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="truncate text-sm font-semibold">{app.name}</span>
                {app.connected ? (
                  <Chip size="sm" className="bg-success/12 text-success">
                    connected
                  </Chip>
                ) : app.pending ? (
                  <Chip size="sm" className="bg-default text-muted">
                    signing in
                  </Chip>
                ) : (
                  <Chip size="sm" className="bg-danger/12 text-danger">
                    {app.status.toLowerCase()}
                  </Chip>
                )}
              </div>
              {!app.connected && (
                <p className="mt-0.5 text-[12.5px] text-muted">
                  {app.pending
                    ? "Finish signing in in your browser, then reopen this panel."
                    : (app.status_reason ?? "Reconnect this app to use it again.")}
                </p>
              )}
            </div>
            <Button
              size="sm"
              variant="ghost"
              isDisabled={busySlug === app.toolkit_slug}
              onPress={() => disconnect(app)}
            >
              {busySlug === app.toolkit_slug ? "Removing…" : "Disconnect"}
            </Button>
          </div>
        ))}
      </div>
    </Overlay>
  );
}

/** The app's own logo where Composio serves one, its initial where it does not. */
function AppMark({ name, logo }: { name: string; logo: string | null }) {
  const [broken, setBroken] = useState(false);
  if (logo && !broken) {
    return (
      <img
        src={logo}
        alt=""
        onError={() => setBroken(true)}
        className="h-8 w-8 shrink-0 rounded-lg object-contain"
      />
    );
  }
  return (
    <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-default text-[13px] font-semibold text-muted">
      {name.slice(0, 1).toUpperCase()}
    </span>
  );
}

// ------------------------------------------------------------------- browse

function BrowseScreen({
  connected,
  onClose,
  onBack,
  onStarted,
}: {
  connected: AppView[];
  onClose: () => void;
  onBack: () => void;
  onStarted: (slug: string, label: string, url: string) => void;
}) {
  const [search, setSearch] = useState("");
  const [results, setResults] = useState<Toolkit[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  // The catalogue is searched on Composio's side, so nothing is held here and
  // no app list is baked into SirVibe.
  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(() => {
      void (async () => {
        try {
          const found = await api.appsCatalog(search);
          if (!cancelled) {
            setResults(found);
            setError(null);
          }
        } catch (e) {
          if (!cancelled) {
            setResults([]);
            setError(String(e));
          }
        }
      })();
    }, search ? 250 : 0);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [search]);

  const connect = async (kit: Toolkit) => {
    setBusy(kit.slug);
    setError(null);
    try {
      const started = await api.appsConnect(kit.slug);
      onStarted(started.toolkit_slug, started.name, started.redirect_url);
    } catch (e) {
      setError(String(e));
      setBusy(null);
    }
  };

  return (
    <Overlay
      title="Add App"
      subtitle="Pick an app and sign in with your own account. SirVibe never sees your password or the app's key."
      onClose={onClose}
      width="max-w-2xl"
      footer={
        <Button variant="secondary" onPress={onBack}>
          Back
        </Button>
      }
    >
      <div className="mb-3 flex items-center gap-2 rounded-lg border border-field-border bg-field px-3">
        <SearchIcon className="h-4 w-4 shrink-0 text-muted" />
        <input
          className="w-full bg-transparent py-2 text-sm text-foreground outline-none"
          placeholder="Search apps — gmail, github, drive, slack…"
          autoFocus
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      {error && (
        <p className="mb-3 rounded-lg bg-danger/10 px-3 py-2 text-[13px] text-danger">{error}</p>
      )}

      {results === null && (
        <div className="flex items-center gap-2 py-8 text-sm text-muted">
          <Spinner className="h-4 w-4" /> Loading apps…
        </div>
      )}

      {results?.length === 0 && !error && (
        <p className="py-8 text-center text-[13px] text-muted">
          Nothing matched “{search}”.
        </p>
      )}

      <div className="flex flex-col gap-2">
        {results?.map((kit) => {
          const already = connected.find((a) => a.toolkit_slug === kit.slug);
          return (
            <div
              key={kit.slug}
              className="flex items-center gap-3 rounded-xl border border-border px-3.5 py-2.5"
            >
              <AppMark name={kit.name} logo={kit.logo} />
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm font-medium">{kit.name}</div>
                <div className="truncate text-[12px] text-muted">
                  {kit.tools_count > 0 ? `${kit.tools_count} actions` : "no actions listed"}
                  {kit.categories.length > 0 && ` · ${kit.categories[0]}`}
                </div>
              </div>
              {already?.connected ? (
                <Chip size="sm" className="bg-success/12 text-success">
                  connected
                </Chip>
              ) : !kit.connectable ? (
                <span
                  className="text-[12px] text-muted"
                  title="This app needs OAuth credentials registered in the Composio dashboard before it can be connected."
                >
                  needs setup
                </span>
              ) : (
                <Button
                  size="sm"
                  variant="secondary"
                  isDisabled={busy === kit.slug}
                  onPress={() => connect(kit)}
                >
                  {busy === kit.slug ? "Opening…" : "Connect"}
                </Button>
              )}
            </div>
          );
        })}
      </div>
    </Overlay>
  );
}

// ------------------------------------------------------------------ waiting

/**
 * The sign-in itself happens in the user's real browser. This screen waits for
 * Composio to report that it finished, and offers the link again in case the
 * browser never came to the front.
 */
function WaitingScreen({
  slug,
  label,
  url,
  onClose,
  onDone,
}: {
  slug: string;
  label: string;
  url: string;
  onClose: () => void;
  onDone: (message: string, failed: boolean) => void;
}) {
  const [message, setMessage] = useState(`Waiting for you to finish signing in to ${label}…`);
  const finished = useRef(false);

  useEffect(() => {
    let tries = 0;
    let timer: ReturnType<typeof setTimeout>;

    const poll = async () => {
      if (finished.current) return;
      tries += 1;
      try {
        const view = await api.appsCheck(slug);
        if (view.connected) {
          finished.current = true;
          onDone(`${view.name} connected.`, false);
          return;
        }
        if (!view.pending) {
          finished.current = true;
          onDone(view.status_reason ?? `Connecting ${view.name} did not complete.`, true);
          return;
        }
      } catch (e) {
        // A transient failure while polling is not a failed connection; keep
        // waiting, but stop pretending everything is fine if it persists.
        if (tries > 4) setMessage(`Still waiting. ${String(e)}`);
      }
      if (tries >= POLL_LIMIT) {
        finished.current = true;
        onDone(
          `${label} did not finish signing in. The link may have expired — try connecting it again.`,
          true,
        );
        return;
      }
      timer = setTimeout(poll, POLL_MS);
    };

    timer = setTimeout(poll, POLL_MS);
    return () => {
      finished.current = true;
      clearTimeout(timer);
    };
  }, [slug, label, onDone]);

  return (
    <Overlay
      title={`Connecting ${label}`}
      subtitle="Sign in to your account in the browser window that just opened."
      onClose={onClose}
      footer={
        <Button variant="secondary" onPress={() => onDone("Connection cancelled.", false)}>
          Cancel
        </Button>
      }
    >
      <div className="flex items-center gap-3 py-6">
        <Spinner className="h-4 w-4" />
        <span className="text-sm text-muted">{message}</span>
      </div>
      <p className="text-[12.5px] text-muted">
        If no browser window opened, paste this link into your browser:
      </p>
      <p className="mt-1 break-all rounded-lg bg-default px-3 py-2 font-mono text-[11.5px] text-muted">
        {url}
      </p>
    </Overlay>
  );
}

// ---------------------------------------------------------------------- key

function KeyScreen({
  status,
  onClose,
  onBack,
  onSaved,
}: {
  status: AppsStatus | null;
  onClose: () => void;
  onBack?: () => void;
  onSaved: (status: AppsStatus) => void;
}) {
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      // The backend verifies the key against Composio before storing it, so a
      // bad key is refused here rather than failing silently later.
      onSaved(await api.appsSetKey(key));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    try {
      onSaved(await api.appsClearKey());
    } finally {
      setBusy(false);
    }
  };

  return (
    <Overlay
      title="Connected apps"
      subtitle="Apps are connected through Composio, which brokers the sign-in so you never paste an app's own API key."
      onClose={onClose}
      footer={
        <>
          {onBack && (
            <Button variant="secondary" onPress={onBack} isDisabled={busy}>
              Back
            </Button>
          )}
          {status?.configured && !status.from_environment && (
            <Button variant="ghost" onPress={clear} isDisabled={busy}>
              Remove key
            </Button>
          )}
          <Button variant="primary" onPress={save} isDisabled={busy || !key.trim()}>
            {busy ? "Checking…" : "Save"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        {status?.from_environment && (
          <p className="rounded-lg bg-accent/[0.08] px-3 py-2 text-[13px] text-foreground">
            A key is being read from the COMPOSIO_API_KEY environment variable. Saving one here
            will take precedence over it.
          </p>
        )}

        <label className="block">
          <span className="mb-1.5 block text-[13px] font-medium text-foreground">
            Composio API key
          </span>
          <div className="flex items-center gap-2">
            <KeyIcon className="h-4 w-4 shrink-0 text-muted" />
            <input
              className={inputClass}
              type="password"
              autoComplete="off"
              spellCheck={false}
              autoFocus
              placeholder={status?.configured ? "••••••••••••••••" : "Paste your Composio API key"}
              value={key}
              onChange={(e) => setKey(e.target.value)}
            />
          </div>
          <span className="mt-1 block text-[11.5px] text-muted">
            {status?.configured
              ? `A key is stored (${status.key_hint}). Enter a new one to replace it.`
              : "Stored on this machine only. It is never sent to the model, shown in the interface again, or written to a log."}
          </span>
        </label>

        <p className="text-[12.5px] text-muted">
          Create one in your Composio dashboard under project settings. It is the only key you
          need — each app you add after that is connected by signing in.
        </p>

        {error && (
          <p className="rounded-lg bg-danger/10 px-3 py-2 text-[13px] text-danger">{error}</p>
        )}
      </div>
    </Overlay>
  );
}
