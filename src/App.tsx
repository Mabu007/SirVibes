import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { Avatar, Button } from "@heroui/react";
import { Agent, type ApprovalRequest } from "./lib/agent";
import { api } from "./lib/api";
import type {
  ConversationSummary,
  SettingsView,
  SystemUsage,
  ToolProgress,
} from "./lib/types";
import type { QuestionRequest } from "./lib/agent";
import { SessionStore } from "./lib/sessions";
import {
  deleteChat as deleteCloudChat,
  loadChats,
  saveChat,
  saveMessages,
  signOutUser,
  watchUser,
  type User,
} from "./lib/firebase";
import { LoginScreen } from "./components/LoginScreen";
import { ArtifactStrip } from "./components/ArtifactStrip";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { Composer } from "./components/Composer";
import { AvatarIcon, CopyIcon, PanelIcon, RetryIcon } from "./components/Icons";
import { Markdown } from "./components/Markdown";
import { ModelPicker } from "./components/ModelPicker";
import { WorkspacesModal } from "./components/WorkspacesModal";
import { SettingsPanel } from "./components/SettingsPanel";
import { SetupModal } from "./components/SetupModal";
import { Sidebar } from "./components/Sidebar";
import { ApisModal } from "./components/ApisModal";
import { AppsModal } from "./components/AppsModal";
import { SkillsModal } from "./components/SkillsModal";
import { ToolCard } from "./components/ToolCard";

const newId = () => `c${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

const EXAMPLES = [
  "Analyse the videos in this folder and tell me what I have.",
  "Turn this podcast into three vertical Shorts.",
  "Generate captions for interview.mp4 and burn them in.",
];

/**
 * What the machine and the agents are doing, at a glance. Beside the profile
 * because that is where a desktop app puts the things about *this* session.
 */
function StatusLine({ active, usage }: { active: number; usage: SystemUsage | null }) {
  return (
    <div className="hidden items-center gap-3 text-xs text-muted sm:flex">
      <span className="flex items-center gap-1.5" title={`${active} agent(s) working`}>
        <span className="relative flex h-2 w-2">
          {active > 0 && (
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-success opacity-75" />
          )}
          <span
            className={`relative inline-flex h-2 w-2 rounded-full ${
              active > 0 ? "bg-success" : "bg-border"
            }`}
          />
        </span>
        {active > 0 ? `${active} agent${active === 1 ? "" : "s"}` : "idle"}
      </span>
      {usage && (
        <>
          <span className="tabular-nums">CPU {Math.round(usage.cpu_percent)}%</span>
          <span className="tabular-nums">RAM {usage.ram_used_gb.toFixed(1)} GB</span>
        </>
      )}
    </div>
  );
}

/**
 * An agent finished while the user was somewhere else. Says so once, quietly,
 * and gets out of the way — it never steals focus or interrupts the chat the
 * user is actually reading.
 */
function DoneToast({
  title,
  onOpen,
  onDismiss,
}: {
  title: string;
  onOpen: () => void;
  onDismiss: () => void;
}) {
  useEffect(() => {
    const timer = setTimeout(onDismiss, 8000);
    return () => clearTimeout(timer);
  }, [onDismiss]);

  return (
    <div className="fixed right-4 bottom-4 z-50 flex items-center gap-3 rounded-xl border border-border bg-background px-3.5 py-2.5 shadow-lg">
      <span className="h-2 w-2 shrink-0 rounded-full bg-success" />
      <span className="text-[13px] text-foreground">
        <span className="font-medium">{title}</span> is done
      </span>
      <Button size="sm" variant="secondary" onPress={onOpen}>
        Open
      </Button>
      <button
        onClick={onDismiss}
        aria-label="Dismiss"
        className="text-muted hover:text-foreground"
      >
        ✕
      </button>
    </div>
  );
}

export default function App() {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [, setTick] = useState(0);
  const [approval, setApproval] = useState<ApprovalRequest | null>(null);
  const [question, setQuestion] = useState<QuestionRequest | null>(null);
  const [panel, setPanel] = useState<
    null | "profile" | "models" | "setup" | "skills" | "workspaces" | "apis" | "apps"
  >(
    null,
  );
  const [collapsed, setCollapsed] = useState(false);
  const [history, setHistory] = useState<ConversationSummary[]>([]);
  const [conversationId, setConversationId] = useState(newId);
  const [draft, setDraft] = useState("");
  const [pendingDelete, setPendingDelete] = useState<ConversationSummary | null>(null);
  const [done, setDone] = useState<{ id: string; title: string } | null>(null);
  const [user, setUser] = useState<User | null>(null);
  const [authReady, setAuthReady] = useState(false);
  const userRef = useRef<User | null>(null);
  userRef.current = user;

  // A prompt belongs to the chat that raised it, so a question in one chat does
  // not appear over another and is still there when you come back to it.
  const resolveApproval = useRef(new Map<string, (approved: boolean) => void>());
  const resolveQuestion = useRef(new Map<string, (answer: string | null) => void>());
  const scroller = useRef<HTMLDivElement>(null);
  const activeIdRef = useRef(conversationId);
  activeIdRef.current = conversationId;

  // Every open chat runs its own agent. Opening a second one starts a second
  // agent beside the first; it does not displace it.
  const storeRef = useRef<SessionStore | null>(null);
  if (!storeRef.current) {
    storeRef.current = new SessionStore(
      (chatId) =>
        new Agent({
          onChange: () => {
            storeRef.current?.refresh(chatId, activeIdRef.current === chatId);
            setTick((t) => t + 1);
          },
          requestApproval: (request) =>
            new Promise<boolean>((resolve) => {
              resolveApproval.current.set(chatId, resolve);
              setApproval({ ...request, chatId });
            }),
          closeApproval: () => {
            resolveApproval.current.delete(chatId);
            setApproval((a) => (a?.chatId === chatId ? null : a));
          },
          askUser: (request) =>
            new Promise<string | null>((resolve) => {
              resolveQuestion.current.set(chatId, resolve);
              setQuestion({ ...request, chatId });
            }),
          // Stopping a run takes its question with it, and releases anything
          // waiting on an answer that is never coming.
          closeQuestion: () => {
            resolveQuestion.current.get(chatId)?.(null);
            resolveQuestion.current.delete(chatId);
            setQuestion((q) => (q?.chatId === chatId ? null : q));
          },
        }),
      () => setTick((t) => t + 1),
    );
  }
  const sessions = storeRef.current;
  const agent = sessions.ensure(conversationId);

  // Gently: the status line is a glance, not a monitor.
  const [usage, setUsage] = useState<SystemUsage | null>(null);
  useEffect(() => {
    let alive = true;
    const read = () => api.systemUsage().then((u) => alive && setUsage(u)).catch(() => {});
    read();
    const timer = setInterval(read, 4000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);

  useEffect(
    () =>
      watchUser((who) => {
        setUser(who);
        setAuthReady(true);
      }),
    [],
  );

  // The chats that belong to this account, as soon as we know whose they are.
  useEffect(() => {
    if (!user) return;
    let alive = true;
    loadChats(user.uid)
      .then((chats) => {
        if (!alive) return;
        setHistory((local) => {
          const seen = new Set(local.map((c) => c.id));
          const fromCloud = chats
            .filter((c) => !seen.has(c.id))
            .map((c) => ({
              id: c.id,
              title: c.title,
              updated_ms: c.updatedMs,
              workspace: c.workspace,
            }));
          return [...local, ...fromCloud].sort((a, b) => b.updated_ms - a.updated_ms);
        });
      })
      .catch(() => {
        // Offline, or rules not yet deployed: local history still works.
      });
    return () => {
      alive = false;
    };
  }, [user]);

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
      // Broadcast: a chat that is not the one this stream or call belongs to
      // ignores it. Routing has to reach agents working out of sight.
      listen<{ stream_id: string; kind: string; text: string }>("agent://delta", (e) =>
        sessions
          .open()
          .forEach((id) =>
            sessions.agent(id)?.applyDelta(e.payload.stream_id, e.payload.kind, e.payload.text),
          ),
      ),
      listen<{ call_id: string; lines: string[] }>("agent://shell-output", (e) =>
        sessions
          .open()
          .forEach((id) => sessions.agent(id)?.applyShellOutput(e.payload.call_id, e.payload.lines)),
      ),
      listen<{ call_id: string } & ToolProgress>("agent://shell-progress", (e) =>
        sessions
          .open()
          .forEach((id) => sessions.agent(id)?.applyShellProgress(e.payload.call_id, e.payload)),
      ),
    ];
    return () => {
      unlisten.forEach((p) => p.then((off) => off()));
    };
  }, [sessions]);

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

  const persist = async (chatId: string) => {
    const chat = sessions.agent(chatId);
    const meta = sessions.meta(chatId);
    if (!chat || !chat.items.length) return;
    await api.saveConversation(chatId, {
      id: chatId,
      title: meta?.title ?? "Untitled",
      updated_ms: Date.now(),
      workspace: settings?.workspace ?? null,
      items: chat.items,
      messages: chat.messages,
    });
    setHistory(await api.listConversations());

    // The shape of the work follows the account; the media stays on the disk
    // that made it.
    const who = userRef.current;
    if (who) {
      try {
        await saveChat(who.uid, {
          id: chatId,
          title: meta?.title ?? "Untitled",
          status: meta?.status ?? "idle",
          workspace: settings?.workspace ?? null,
          updatedMs: Date.now(),
        });
        await saveMessages(who.uid, chatId, chat.messages as { role: string; content: string }[]);
      } catch {
        // Never let a sync problem lose the local save that already happened.
      }
    }
  };

  /**
   * A name for the chat that says what it is for, rather than the first
   * seventy characters of what was typed. Generated once, and never over a
   * name the user chose.
   */
  const nameChat = async (chatId: string, request: string) => {
    const meta = sessions.meta(chatId);
    if (!meta || meta.titleLocked) return;
    sessions.suggestTitle(chatId, request.trim().split(/\s+/).slice(0, 5).join(" "));
    try {
      const title = await api.generateText(
        request.slice(0, 500),
        "Name this piece of work in two to five words, as a person would label it in a list. " +
          "Reply with the name and nothing else. No quotes, no punctuation at the end, no verbs " +
          "like 'help' or 'please'. Examples: Instagram Growth Analysis. Podcast Shorts. " +
          "Client Proposal PDF.",
      );
      const clean = title.replace(/^["'\s]+|["'\s.]+$/g, "").split("\n")[0];
      if (clean) sessions.suggestTitle(chatId, clean);
    } catch {
      // The fallback name is already in place; a title is not worth an error.
    }
  };

  /**
   * Send into one chat and let it run. Deliberately not awaited by the caller:
   * the user is free to open another chat and start another agent while this
   * one works.
   */
  const send = async (text: string) => {
    const chatId = conversationId;
    const chat = sessions.ensure(chatId);
    const first = chat.items.length === 0;
    if (first) void nameChat(chatId, text);
    await chat.send(text);
    await persist(chatId);
    if (activeIdRef.current !== chatId) {
      const meta = sessions.meta(chatId);
      if (meta) setDone({ id: chatId, title: meta.title });
    }
  };

  const answer = (choice: string | null) => {
    const chatId = question?.chatId;
    if (!chatId) return;
    resolveQuestion.current.get(chatId)?.(choice);
    resolveQuestion.current.delete(chatId);
    setQuestion(null);
  };

  const decide = (approved: boolean) => {
    const chatId = approval?.chatId;
    if (!chatId) return;
    resolveApproval.current.get(chatId)?.(approved);
    resolveApproval.current.delete(chatId);
    setApproval(null);
  };

  const chooseWorkspace = async () => {
    const picked = await open({ directory: true, multiple: false, title: "Choose workspace folder" });
    if (typeof picked === "string") setSettings(await api.updateSettings({ workspace: picked }));
  };

  const startNew = () => {
    const id = newId();
    sessions.ensure(id);
    setConversationId(id);
    setDraft("");
  };

  /**
   * Switch to another chat. Whatever the current one is doing, it carries on:
   * this only changes what is on screen.
   */
  const openConversation = async (id: string) => {
    if (!sessions.agent(id)) {
      const conversation = await api.loadConversation(id);
      const restored = sessions.ensure(id, conversation.title || "Untitled", true);
      restored.load(conversation);
    }
    sessions.read(id);
    setConversationId(id);
  };

  const removeConversation = async (id: string) => {
    await api.deleteConversation(id);
    if (userRef.current) {
      try {
        await deleteCloudChat(userRef.current.uid, id);
      } catch {
        // The local delete is what matters; the account copy can lag.
      }
    }
    sessions.close(id);
    setHistory(await api.listConversations());
    if (id === conversationId) startNew();
    setPendingDelete(null);
  };

  const renameChat = (id: string, title: string) => {
    sessions.rename(id, title);
    void persist(id);
  };

  /** Re-run the last thing you asked for, discarding the reply after it. */
  const retry = async () => {
    const lastUser = [...agent.items].reverse().find((i) => i.kind === "user");
    if (!lastUser || lastUser.kind !== "user" || agent.running) return;
    agent.rewindToLastUser();
    await send(lastUser.text);
  };

  if (!authReady) {
    return <div className="grid h-full place-items-center text-sm text-muted">Starting…</div>;
  }
  if (!user) return <LoginScreen />;

  if (!settings) {
    return <div className="grid h-full place-items-center text-sm text-muted">Starting…</div>;
  }

  const needsSetup = !settings.api_key_set || !settings.model || !settings.workspace;
  const empty = agent.items.length === 0;
  const title = sessions.meta(conversationId)?.title ?? "New chat";

  // Open chats first — those are the ones with an agent inside them — then the
  // saved ones that are not open, so nothing is lost from before.
  const openChats = sessions.list().map((meta) => ({
    id: meta.id,
    title: meta.title,
    status: meta.status,
    unread: meta.unread,
    open: true,
  }));
  const openIds = new Set(openChats.map((c) => c.id));
  const chats = [
    ...openChats,
    ...history
      .filter((h) => !openIds.has(h.id))
      .map((h) => ({
        id: h.id,
        title: h.title || "Untitled",
        status: "idle" as const,
        unread: false,
        open: false,
      })),
  ];

  return (
    <div className="flex h-full bg-background text-foreground">
      <Sidebar
        chats={chats}
        currentId={conversationId}
        workspace={settings.workspace}
        collapsed={collapsed}
        onToggle={() => setCollapsed(true)}
        onNew={startNew}
        onOpen={openConversation}
        onDelete={(id) =>
          setPendingDelete(
            history.find((h) => h.id === id) ?? {
              id,
              title: sessions.meta(id)?.title ?? "this chat",
              updated_ms: Date.now(),
              workspace: null,
            },
          )
        }
        onRename={renameChat}
        onSkills={() => setPanel("skills")}
        onWorkspaces={() => setPanel("workspaces")}
        onApis={() => setPanel("apis")}
        onApps={() => setPanel("apps")}
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
          <StatusLine active={sessions.activeCount()} usage={usage} />
          {/* The account control, where a desktop app keeps it: top right, the
              user's own corner of the window. */}
          <Button
            variant="ghost"
            size="sm"
            isIconOnly
            aria-label="Profile"
            onPress={() => setPanel("profile")}
            className="h-9 w-9 rounded-full border border-border text-muted hover:text-foreground"
          >
            <AvatarIcon className="h-[18px] w-[18px]" />
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
                        askingHere={question?.itemId === item.id}
                        onAnswer={answer}
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
          mode={settings.permission_mode}
          onMode={async (permission_mode) =>
            setSettings(await api.updateSettings({ permission_mode }))
          }
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
      {done && (
        <DoneToast
          title={done.title}
          onOpen={() => {
            void openConversation(done.id);
            setDone(null);
          }}
          onDismiss={() => setDone(null)}
        />
      )}

      {panel === "profile" && (
        <SettingsPanel
          account={{ email: user.email ?? "Signed in", onSignOut: () => void signOutUser() }}
          settings={settings}
          onSettings={setSettings}
          onClose={() => setPanel(null)}
        />
      )}
      {panel === "skills" && (
        <SkillsModal settings={settings} onSettings={setSettings} onClose={() => setPanel(null)} />
      )}
      {panel === "apis" && <ApisModal onClose={() => setPanel(null)} />}
      {panel === "apps" && <AppsModal onClose={() => setPanel(null)} />}
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
