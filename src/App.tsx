import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { Avatar, Button } from "@heroui/react";
import { Agent, type ApprovalRequest } from "./lib/agent";
import { api } from "./lib/api";
import type { ConversationSummary, PermissionMode, SettingsView } from "./lib/types";
import { ArtifactStrip } from "./components/ArtifactStrip";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { Composer } from "./components/Composer";
import { CopyIcon, GearIcon, PanelIcon, RetryIcon } from "./components/Icons";
import { Markdown } from "./components/Markdown";
import { ModelPicker } from "./components/ModelPicker";
import { WorkspacesModal } from "./components/WorkspacesModal";
import { SettingsPanel } from "./components/SettingsPanel";
import { SetupModal } from "./components/SetupModal";
import { Sidebar } from "./components/Sidebar";
import { ApisModal } from "./components/ApisModal";
import { SkillsModal } from "./components/SkillsModal";
import { ToolCard } from "./components/ToolCard";

const newId = () => `c${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

const MODE_LABEL: Record<PermissionMode, string> = {
  ask: "Ask every time",
  smart: "Smart",
  full: "Full autonomy",
};

const EXAMPLES = [
  "Analyse the videos in this folder and tell me what I have.",
  "Turn this podcast into three vertical Shorts.",
  "Generate captions for interview.mp4 and burn them in.",
];

export default function App() {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [, setTick] = useState(0);
  const [approval, setApproval] = useState<ApprovalRequest | null>(null);
  const [panel, setPanel] = useState<null | "settings" | "models" | "setup" | "skills" | "workspaces" | "apis">(
    null,
  );
  const [collapsed, setCollapsed] = useState(false);
  const [history, setHistory] = useState<ConversationSummary[]>([]);
  const [conversationId, setConversationId] = useState(newId);
  const [draft, setDraft] = useState("");
  const [pendingDelete, setPendingDelete] = useState<ConversationSummary | null>(null);

  const resolveApproval = useRef<((approved: boolean) => void) | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const agentRef = useRef<Agent | null>(null);

  if (!agentRef.current) {
    agentRef.current = new Agent({
      onChange: () => setTick((t) => t + 1),
      requestApproval: (request) =>
        new Promise<boolean>((resolve) => {
          resolveApproval.current = resolve;
          setApproval(request);
        }),
    });
  }
  const agent = agentRef.current;

  useEffect(() => {
    api.getSettings().then((s) => {
      setSettings(s);
      // First run: show what is needed, without ever disabling the chat.
      if (!s.api_key_set || !s.model || !s.workspace) setPanel("setup");
    });
    api.listConversations().then(setHistory);
  }, []);

  useEffect(() => {
    const unlisten = [
      listen<{ stream_id: string; kind: string; text: string }>("agent://delta", (e) =>
        agent.applyDelta(e.payload.stream_id, e.payload.kind, e.payload.text),
      ),
      listen<{ call_id: string; line: string }>("agent://shell-output", (e) =>
        agent.applyShellOutput(e.payload.call_id, e.payload.line),
      ),
    ];
    return () => {
      unlisten.forEach((p) => p.then((off) => off()));
    };
  }, [agent]);

  // Follow the conversation unless the user has scrolled up to read something.
  // A ResizeObserver keeps it pinned when content grows after render too —
  // streamed text, expanding output, and video that reports its size late.
  const pinned = useRef(true);
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const atBottom = () => el.scrollHeight - el.scrollTop - el.clientHeight < 220;
    const onScroll = () => {
      pinned.current = atBottom();
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    const observer = new ResizeObserver(() => {
      if (pinned.current) el.scrollTop = el.scrollHeight;
    });
    if (el.firstElementChild) observer.observe(el.firstElementChild);
    return () => {
      el.removeEventListener("scroll", onScroll);
      observer.disconnect();
    };
  }, []);

  useEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  });

  const persist = async () => {
    if (!agent.items.length) return;
    const firstUser = agent.items.find((i) => i.kind === "user");
    await api.saveConversation(conversationId, {
      id: conversationId,
      title: firstUser && firstUser.kind === "user" ? firstUser.text.slice(0, 70) : "Untitled",
      updated_ms: Date.now(),
      workspace: settings?.workspace ?? null,
      items: agent.items,
      messages: agent.messages,
    });
    setHistory(await api.listConversations());
  };

  const send = async (text: string) => {
    await agent.send(text);
    await persist();
  };

  const decide = (approved: boolean) => {
    resolveApproval.current?.(approved);
    resolveApproval.current = null;
    setApproval(null);
  };

  const chooseWorkspace = async () => {
    const picked = await open({ directory: true, multiple: false, title: "Choose workspace folder" });
    if (typeof picked === "string") setSettings(await api.updateSettings({ workspace: picked }));
  };

  const startNew = () => {
    agent.reset();
    setConversationId(newId());
    setDraft("");
  };

  const openConversation = async (id: string) => {
    const conversation = await api.loadConversation(id);
    agent.load(conversation);
    setConversationId(id);
  };

  const removeConversation = async (id: string) => {
    await api.deleteConversation(id);
    setHistory(await api.listConversations());
    if (id === conversationId) startNew();
    setPendingDelete(null);
  };

  /** Re-run the last thing you asked for, discarding the reply after it. */
  const retry = async () => {
    const lastUser = [...agent.items].reverse().find((i) => i.kind === "user");
    if (!lastUser || lastUser.kind !== "user" || agent.running) return;
    agent.rewindToLastUser();
    await send(lastUser.text);
  };

  if (!settings) {
    return <div className="grid h-full place-items-center text-sm text-muted">Starting…</div>;
  }

  const needsSetup = !settings.api_key_set || !settings.model || !settings.workspace;
  const empty = agent.items.length === 0;
  const title = history.find((h) => h.id === conversationId)?.title ?? "New chat";

  return (
    <div className="flex h-full bg-background text-foreground">
      <Sidebar
        history={history}
        currentId={conversationId}
        workspace={settings.workspace}
        collapsed={collapsed}
        onToggle={() => setCollapsed(true)}
        onNew={startNew}
        onOpen={openConversation}
        onDelete={(id) =>
          setPendingDelete(history.find((h) => h.id === id) ?? null)
        }
        onSkills={() => setPanel("skills")}
        onWorkspaces={() => setPanel("workspaces")}
        onApis={() => setPanel("apis")}
        onChooseFolder={chooseWorkspace}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center gap-3 border-b border-border px-4 py-2.5">
          {collapsed && (
            <Button variant="ghost" size="sm" isIconOnly aria-label="Show sidebar" onPress={() => setCollapsed(false)}>
              <PanelIcon />
            </Button>
          )}
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-semibold">{title}</div>
            <div className="truncate text-xs text-muted">
              {settings.workspace ?? "No workspace selected"}
            </div>
          </div>
          <select
            value={settings.permission_mode}
            onChange={async (e) =>
              setSettings(
                await api.updateSettings({ permission_mode: e.target.value as PermissionMode }),
              )
            }
            className={`rounded-lg border px-2.5 py-1.5 text-[13px] outline-none ${
              settings.permission_mode === "full"
                ? "border-warning/50 bg-warning/10 text-warning-foreground"
                : "border-border bg-background"
            }`}
          >
            {(Object.keys(MODE_LABEL) as PermissionMode[]).map((m) => (
              <option key={m} value={m}>
                {MODE_LABEL[m]}
              </option>
            ))}
          </select>
          <Button variant="secondary" size="sm" onPress={() => setPanel("settings")}>
            <GearIcon />
            Settings
          </Button>
        </header>

        <div ref={scroller} className={`min-h-0 flex-1 overflow-y-auto ${empty ? "flex flex-col justify-center" : ""}`}>
          <div className="mx-auto w-full max-w-3xl px-4 py-6">
            {empty ? (
              <div className="text-center">
                <h1 className="text-2xl font-semibold tracking-tight">What are we making today?</h1>
                <p className="mx-auto mt-2 max-w-md text-sm text-muted">
                  Describe the outcome. The agent uses the programs on this computer and the
                  APIs you have connected to produce it in your workspace.
                </p>
                <div className="mx-auto mt-6 flex max-w-lg flex-col gap-2">
                  {EXAMPLES.map((e) => (
                    <button
                      key={e}
                      onClick={() => setDraft(e)}
                      className="rounded-xl border border-border px-3.5 py-2.5 text-left text-[13.5px] text-foreground transition-colors hover:bg-default/60"
                    >
                      {e}
                    </button>
                  ))}
                </div>
              </div>
            ) : (
              agent.items.map((item, index) => {
                switch (item.kind) {
                  case "user":
                    return (
                      <div key={item.id} className="mb-5 flex justify-end">
                        <div className="max-w-[80%] rounded-2xl bg-default px-4 py-2.5 text-[15px] whitespace-pre-wrap">
                          {item.text}
                        </div>
                      </div>
                    );
                  case "assistant":
                    return (
                      <div key={item.id} className="mb-5 flex gap-3">
                        <Avatar size="sm" className="mt-0.5 shrink-0 bg-default">
                          <Avatar.Fallback className="text-[10px] font-semibold">AI</Avatar.Fallback>
                        </Avatar>
                        <div className="min-w-0 flex-1">
                          {item.reasoning && (
                            <details className="mb-2">
                              <summary className="cursor-pointer text-[13px] text-muted">
                                Thought for a moment
                              </summary>
                              <div className="mt-1.5 border-l-2 border-border pl-3 text-[13px] whitespace-pre-wrap text-muted">
                                {item.reasoning}
                              </div>
                            </details>
                          )}
                          <Markdown text={item.text} />
                          {item.streaming && !item.text && (
                            <span className="inline-block h-4 w-1.5 animate-pulse bg-muted align-middle" />
                          )}
                          {!item.streaming && item.text && isLastAssistant(agent.items, index) && (
                            <div className="mt-2 flex gap-1">
                              <Button
                                variant="ghost"
                                size="sm"
                                isIconOnly
                                aria-label="Copy reply"
                                onPress={() => navigator.clipboard.writeText(item.text)}
                              >
                                <CopyIcon className="h-3.5 w-3.5" />
                              </Button>
                              <Button
                                variant="ghost"
                                size="sm"
                                isIconOnly
                                aria-label="Try again"
                                onPress={retry}
                              >
                                <RetryIcon className="h-3.5 w-3.5" />
                              </Button>
                            </div>
                          )}
                        </div>
                      </div>
                    );
                  case "tool":
                    return (
                      <ToolCard
                        key={item.id}
                        item={item}
                        awaitingHere={approval?.itemId === item.id}
                        onDecide={decide}
                      />
                    );
                  case "artifacts":
                    return <ArtifactStrip key={item.id} items={item.items} />;
                  case "error":
                    return (
                      <div
                        key={item.id}
                        className="my-3 rounded-xl border border-danger/40 bg-danger/[0.06] px-3.5 py-2.5 text-[13.5px] text-danger"
                      >
                        {item.text}
                      </div>
                    );
                }
              })
            )}
          </div>
        </div>

        <Composer
          value={draft}
          onChange={setDraft}
          onSend={send}
          onStop={() => agent.cancel()}
          running={agent.running}
          model={settings.model}
          onPickModel={() => setPanel("models")}
          needsSetup={needsSetup}
          onNeedsSetup={() => setPanel("setup")}
        />
      </div>

      {panel === "setup" && (
        <SetupModal
          settings={settings}
          onSettings={setSettings}
          onPickModel={() => setPanel("models")}
          onClose={() => setPanel(null)}
        />
      )}
      {panel === "models" && (
        <ModelPicker
          current={settings.model}
          onClose={() => setPanel(null)}
          onPick={async (id) => {
            setSettings(await api.updateSettings({ model: id }));
            setPanel(null);
          }}
        />
      )}
      {panel === "settings" && (
        <SettingsPanel
          settings={settings}
          onSettings={setSettings}
          onClose={() => setPanel(null)}
        />
      )}
      {panel === "skills" && (
        <SkillsModal settings={settings} onSettings={setSettings} onClose={() => setPanel(null)} />
      )}
      {panel === "apis" && <ApisModal onClose={() => setPanel(null)} />}
      {panel === "workspaces" && (
        <WorkspacesModal
          settings={settings}
          onSettings={setSettings}
          onClose={() => setPanel(null)}
        />
      )}
      {pendingDelete && (
        <ConfirmDialog
          title="Delete this chat?"
          body={`“${pendingDelete.title || "Untitled"}” and everything in it will be removed. This cannot be undone.`}
          onConfirm={() => removeConversation(pendingDelete.id)}
          onCancel={() => setPendingDelete(null)}
        />
      )}
    </div>
  );
}

function isLastAssistant(items: { kind: string }[], index: number): boolean {
  for (let i = items.length - 1; i > index; i--) {
    if (items[i].kind === "assistant") return false;
  }
  return true;
}
