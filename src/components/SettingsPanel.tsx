import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Button, Chip } from "@heroui/react";
import { api } from "../lib/api";
import type { AppView, Capability, PermissionMode, SettingsView } from "../lib/types";
import { Overlay } from "./Overlay";
import { AvatarIcon } from "./Icons";

const MODES: { value: PermissionMode; label: string; blurb: string }[] = [
  {
    value: "ask",
    label: "Ask every time",
    blurb: "Every tool call waits for your approval before it runs.",
  },
  {
    value: "smart",
    label: "Smart",
    blurb:
      "Routine production work runs immediately. Deleting files, installing software, uploading data and anything outside the workspace ask first.",
  },
  {
    value: "full",
    label: "Full autonomy",
    blurb:
      "The agent runs unattended inside the workspace, including destructive commands. It still asks before reaching outside it.",
  },
];

export function SettingsPanel({
  account,
  settings,
  onSettings,
  onClose,
}: {
  account: {
    email: string;
    /** Whether chats are reaching the account, or only this machine. */
    cloud: "unknown" | "synced" | "local";
    onSignOut: () => void;
  };
  settings: SettingsView;
  onSettings: (s: SettingsView) => void;
  onClose: () => void;
}) {
  const [caps, setCaps] = useState<Capability[]>([]);
  const [apps, setApps] = useState<AppView[]>([]);

  useEffect(() => {
    api.listCapabilities().then(setCaps);
    // The accounts this profile is signed in to already live in the apps
    // registry; this is a read of that, not a second copy of it.
    api.appsList().then(setApps).catch(() => setApps([]));
  }, []);

  const patch = async (p: Parameters<typeof api.updateSettings>[0]) => {
    onSettings(await api.updateSettings(p));
  };

  const chooseWorkspace = async () => {
    const picked = await open({ directory: true, multiple: false, title: "Choose workspace folder" });
    if (typeof picked === "string") await patch({ workspace: picked });
  };

  const connected = apps.filter((a) => a.connected);

  return (
    <Overlay
      title="Profile"
      onClose={onClose}
      width="max-w-xl"
      footer={
        <Button variant="primary" onPress={onClose}>
          Done
        </Button>
      }
    >
      <Section label="Account">
        <div className="flex items-center gap-3">
          <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full border border-border bg-background-secondary text-muted">
            <AvatarIcon className="h-5 w-5" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-medium text-foreground">{account.email}</div>
            <div className="truncate text-xs text-muted">
              {connected.length
                ? `Apps signed in: ${connected.map((a) => a.name).join(", ")}`
                : "No apps connected yet"}
            </div>
            <div className="mt-0.5 flex items-center gap-1.5 text-xs">
              <span
                className={`h-1.5 w-1.5 rounded-full ${
                  account.cloud === "synced"
                    ? "bg-success"
                    : account.cloud === "local"
                      ? "bg-warning"
                      : "bg-border"
                }`}
              />
              <span className="text-muted">
                {account.cloud === "synced"
                  ? "Chats synced to your account"
                  : account.cloud === "local"
                    ? "Saved on this computer only — cloud sync unavailable"
                    : "Chats saved on this computer"}
              </span>
            </div>
          </div>
          <Button size="sm" variant="secondary" onPress={account.onSignOut}>
            Log out
          </Button>
        </div>
        <p className="mt-2.5 text-xs text-muted">
          Your chats follow this account. Your keys, your workspace and everything you make stay on
          this computer. Apps like Gmail or Instagram are signed in separately — you can connect or
          disconnect any of them in the Apps panel.
        </p>
      </Section>

      <Section label="OpenRouter">
        <KeyField
          placeholder="sk-or-v1-…"
          isSet={settings.api_key_set}
          hint={settings.api_key_hint}
          onSave={(key) => patch({ api_key: key })}
        />
        <p className="mt-1.5 text-xs text-muted">
          Runs the agent, and any model you name with <em>run_model</em> for voiceover, images or
          generated clips.
        </p>
        <Row label="Model">
          <code className="font-mono text-[12.5px]">{settings.model || "none selected"}</code>
        </Row>
        <Row label="Vision model">
          <VisionModelField
            value={settings.vision_model}
            onSave={(vision_model) => patch({ vision_model })}
          />
        </Row>
        <Row label="Reference model">
          <VisionModelField
            value={settings.reference_model}
            onSave={(reference_model) => patch({ reference_model })}
          />
        </Row>
        <p className="mt-1.5 text-xs text-muted">
          Every time the agent looks at something — a reference you gave it, a frame of your
          footage, a render it just made — it runs on this model, whatever model is driving the
          agent itself. Clear it to go back to the default. The reference model is the one that
          watches a video you link to — a YouTube reference is watched where it is, never
          downloaded.
        </p>
      </Section>

      <Section label="Deepgram">
        <KeyField
          placeholder="Paste your Deepgram key"
          isSet={settings.deepgram_key_set}
          hint={settings.deepgram_key_hint}
          onSave={(key) => patch({ deepgram_api_key: key })}
        />
        <p className="mt-1.5 text-xs text-muted">
          {settings.deepgram_key_set
            ? "Transcription and voiceover use this automatically — just ask for a transcript or a VO."
            : "Add a key and the agent transcribes and reads scripts on its own. Without it, ask for a transcript and it will tell you the key is missing."}
        </p>
      </Section>

      <Section label="Workspace">
        <div className="flex items-center gap-2">
          <code className="min-w-0 flex-1 truncate rounded-lg bg-background-secondary px-2.5 py-1.5 font-mono text-[12.5px]">
            {settings.workspace ?? "none selected"}
          </code>
          <Button variant="secondary" size="sm" onPress={chooseWorkspace}>
            {settings.workspace ? "Change" : "Choose"}
          </Button>
        </div>
      </Section>

      <Section label="Permissions">
        <div className="flex flex-col gap-2">
          {MODES.map((m) => (
            <label
              key={m.value}
              className={`flex cursor-pointer gap-3 rounded-xl border px-3.5 py-3 transition-colors ${
                settings.permission_mode === m.value
                  ? "border-accent bg-accent/[0.05]"
                  : "border-border hover:bg-default/50"
              }`}
            >
              <input
                type="radio"
                name="mode"
                className="mt-1 accent-accent"
                checked={settings.permission_mode === m.value}
                onChange={() => patch({ permission_mode: m.value })}
              />
              <span>
                <span className="block text-sm font-medium text-foreground">{m.label}</span>
                <span className="mt-0.5 block text-[12.5px] leading-snug text-muted">
                  {m.blurb}
                </span>
              </span>
            </label>
          ))}
        </div>
        <Row label="Command timeout">
          <input
            type="number"
            min={5}
            max={7200}
            value={settings.shell_timeout_secs}
            onChange={(e) => patch({ shell_timeout_secs: Number(e.target.value) })}
            className="w-24 rounded-lg border border-field-border bg-field px-2.5 py-1 text-sm outline-none focus:border-accent"
          />
          <span className="text-xs text-muted">seconds</span>
        </Row>
      </Section>

      <Section label="Detected on this computer">
        <div className="flex flex-wrap gap-1.5">
          {caps.map((c) => (
            <Chip
              key={c.name}
              size="sm"
              className={
                c.available
                  ? "bg-success/10 font-mono text-success"
                  : "bg-default font-mono text-muted line-through"
              }
            >
              {c.name}
            </Chip>
          ))}
        </div>
        <p className="mt-2 text-xs text-muted">
          The agent drives these through the shell. Anything else installed here is available to it
          too.
        </p>
      </Section>
    </Overlay>
  );
}

/** The one model the user does not pick per request: it is how the agent sees. */
function VisionModelField({
  value,
  onSave,
}: {
  value: string;
  onSave: (model: string) => Promise<void>;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);

  const commit = () => {
    if (draft.trim() !== value) void onSave(draft.trim());
  };

  return (
    <input
      value={draft}
      spellCheck={false}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && commit()}
      className="min-w-0 flex-1 rounded-lg border border-field-border bg-field px-2.5 py-1 font-mono text-[12.5px] outline-none focus:border-accent"
    />
  );
}

/** Keys are write-only from here: you can replace one, never read one back. */
function KeyField({
  placeholder,
  isSet,
  hint,
  onSave,
}: {
  placeholder: string;
  isSet: boolean;
  hint: string;
  onSave: (key: string) => Promise<void>;
}) {
  const [value, setValue] = useState("");
  const [saved, setSaved] = useState(false);

  return (
    <div className="flex gap-2">
      <input
        type="password"
        autoComplete="off"
        spellCheck={false}
        placeholder={isSet ? `Key saved ${hint}` : placeholder}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        className="min-w-0 flex-1 rounded-lg border border-field-border bg-field px-3 py-1.5 text-sm outline-none focus:border-accent"
      />
      <Button
        variant="secondary"
        size="sm"
        isDisabled={!value.trim()}
        onPress={async () => {
          await onSave(value);
          setValue("");
          setSaved(true);
          setTimeout(() => setSaved(false), 2000);
        }}
      >
        {saved ? "Saved" : isSet ? "Replace" : "Save key"}
      </Button>
    </div>
  );
}

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <section className="border-b border-border py-4 last:border-b-0">
      <h3 className="mb-2.5 text-xs font-semibold tracking-wide text-muted uppercase">{label}</h3>
      {children}
    </section>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="mt-3 flex flex-wrap items-center gap-2">
      <span className="min-w-[120px] text-[13px] text-muted">{label}</span>
      {children}
    </div>
  );
}
