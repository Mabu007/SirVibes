import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Button } from "@heroui/react";
import { api } from "../lib/api";
import type { Capability, SettingsView } from "../lib/types";
import { Overlay } from "./Overlay";
import { CheckIcon } from "./Icons";

/**
 * Setup lives in a modal so the chat behind it is never disabled. Everything
 * here is optional to complete now — the composer stays usable either way.
 */
export function SetupModal({
  settings,
  onSettings,
  onPickModel,
  onClose,
}: {
  settings: SettingsView;
  onSettings: (s: SettingsView) => void;
  onPickModel: () => void;
  onClose: () => void;
}) {
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [caps, setCaps] = useState<Capability[] | null>(null);

  useEffect(() => {
    api.listCapabilities().then(setCaps).catch(() => setCaps([]));
  }, []);

  // What the agent genuinely cannot work without, as opposed to what is merely
  // useful. Everything else is a bonus and is not worth alarming anyone about.
  const REQUIRED: Record<string, string> = {
    ffmpeg: "Encodes and renders video",
    ffprobe: "Reads what is inside a media file",
    node: "Runs the caption renderer",
  };
  const have = new Set((caps ?? []).filter((c) => c.available).map((c) => c.name));
  const missing = Object.keys(REQUIRED).filter((name) => caps && !have.has(name));

  const saveKey = async () => {
    if (!apiKey.trim()) return;
    setBusy(true);
    try {
      onSettings(await api.updateSettings({ api_key: apiKey }));
      setApiKey("");
    } finally {
      setBusy(false);
    }
  };

  const pickWorkspace = async () => {
    const picked = await open({ directory: true, multiple: false, title: "Choose workspace folder" });
    if (typeof picked === "string") onSettings(await api.updateSettings({ workspace: picked }));
  };

  const steps = [
    {
      done: Boolean(caps) && missing.length === 0,
      title: "System check",
      body: !caps ? (
        <p className="text-[13px] text-muted">Checking this computer…</p>
      ) : (
        <div className="flex flex-col gap-1.5">
          {Object.entries(REQUIRED).map(([name, what]) => (
            <div key={name} className="flex items-center gap-2 text-[13px]">
              <span className={have.has(name) ? "text-success" : "text-danger"}>
                {have.has(name) ? "✓" : "✕"}
              </span>
              <span className="font-medium text-foreground">{name}</span>
              <span className="text-muted">{what}</span>
            </div>
          ))}
          {missing.length > 0 && (
            <div className="mt-1.5 rounded-lg bg-warning/10 px-3 py-2 text-[12.5px] text-warning-foreground">
              <p className="font-medium">
                {missing.join(" and ")} {missing.length === 1 ? "is" : "are"} missing. Video work
                will fail until {missing.length === 1 ? "it is" : "they are"} installed.
              </p>
              <p className="mt-1.5 text-muted">
                macOS: <code>brew install ffmpeg node</code>
                <br />
                Windows: <code>winget install ffmpeg OpenJS.NodeJS</code>
                <br />
                Linux: <code>sudo apt install ffmpeg nodejs</code>
              </p>
              <p className="mt-1.5 text-muted">
                Everything else — chat, files, APIs, connected apps — works without them.
              </p>
            </div>
          )}
        </div>
      ),
    },
    {
      done: settings.api_key_set,
      title: "OpenRouter key",
      body: settings.api_key_set ? (
        <p className="text-[13px] text-muted">
          Saved {settings.api_key_hint} · stored locally, never exposed to the interface
        </p>
      ) : (
        <div className="flex gap-2">
          <input
            type="password"
            placeholder="sk-or-v1-…"
            value={apiKey}
            autoFocus
            onChange={(e) => setApiKey(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && saveKey()}
            className="min-w-0 flex-1 rounded-lg border border-field-border bg-field px-3 py-1.5 text-sm text-foreground outline-none focus:border-accent"
          />
          <Button variant="primary" size="sm" isDisabled={!apiKey.trim() || busy} onPress={saveKey}>
            Save
          </Button>
        </div>
      ),
    },
    {
      done: Boolean(settings.model),
      title: "Model",
      body: (
        <div className="flex items-center gap-2">
          <code className="min-w-0 flex-1 truncate rounded-lg bg-background-secondary px-2.5 py-1.5 font-mono text-[12.5px]">
            {settings.model || "none selected"}
          </code>
          <Button variant="secondary" size="sm" onPress={onPickModel}>
            {settings.model ? "Change" : "Choose"}
          </Button>
        </div>
      ),
    },
    {
      done: Boolean(settings.workspace),
      title: "Workspace",
      body: (
        <div className="flex items-center gap-2">
          <code className="min-w-0 flex-1 truncate rounded-lg bg-background-secondary px-2.5 py-1.5 font-mono text-[12.5px]">
            {settings.workspace ?? "none selected"}
          </code>
          <Button variant="secondary" size="sm" onPress={pickWorkspace}>
            {settings.workspace ? "Change" : "Choose"}
          </Button>
        </div>
      ),
    },
  ];

  const ready = steps.every((s) => s.done);

  return (
    <Overlay
      title="Three things and it can work"
      subtitle="The agent runs ffmpeg and everything else on this computer, inside the folder you choose."
      onClose={onClose}
      footer={
        <>
          <Button variant="secondary" onPress={onClose}>
            {ready ? "Close" : "Later"}
          </Button>
          <Button variant="primary" isDisabled={!ready} onPress={onClose}>
            Start working
          </Button>
        </>
      }
    >
      <ol className="flex flex-col gap-2.5">
        {steps.map((s, i) => (
          <li
            key={s.title}
            className={`rounded-xl border px-3.5 py-3 ${
              s.done ? "border-success/40 bg-success/[0.05]" : "border-border"
            }`}
          >
            <div className="mb-2 flex items-center gap-2.5">
              <span
                className={`flex h-5 w-5 items-center justify-center rounded-full text-[11px] font-semibold ${
                  s.done ? "bg-success text-success-foreground" : "bg-default text-muted"
                }`}
              >
                {s.done ? <CheckIcon className="h-3 w-3" /> : i + 1}
              </span>
              <span className="text-sm font-medium text-foreground">{s.title}</span>
            </div>
            {s.body}
          </li>
        ))}
      </ol>
    </Overlay>
  );
}
