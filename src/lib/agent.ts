import { api } from "./api";
import { deliverables } from "./deliverables";
import type { SessionStatus } from "./sessions";
import type {
  Artifact,
  ChatMessage,
  Conversation,
  Evaluation,
  Item,
  Question,
  ToolCall,
  ToolProgress,
} from "./types";

/** Safety net against a model that loops forever; the user can always continue. */
const MAX_STEPS = 60;

export interface AgentHooks {
  onChange: () => void;
  /** Resolve true to run the action, false to refuse it. */
  requestApproval: (request: ApprovalRequest) => Promise<boolean>;
  /** Take the approval prompt down: the run it belonged to is over. */
  closeApproval: () => void;
  /**
   * Put a question to the user and wait. Resolves with what they chose, or
   * null if they dismissed it without answering.
   */
  askUser: (request: QuestionRequest) => Promise<string | null>;
  /** Take the question down: the run it belonged to is over. */
  closeQuestion: () => void;
}

export interface QuestionRequest {
  itemId: string;
  question: Question;
  /** Which chat is asking. Filled in by the app when it raises the prompt. */
  chatId?: string;
}

export interface ApprovalRequest {
  itemId: string;
  tool: string;
  args: Record<string, unknown>;
  evaluation: Evaluation;
  /** Which chat is asking. Filled in by the app when it raises the prompt. */
  chatId?: string;
}

const uid = () => Math.random().toString(36).slice(2, 10);

export class Agent {
  items: Item[] = [];
  messages: ChatMessage[] = [];
  running = false;
  error: string | null = null;

  /** True while the run is stopped at a prompt only the user can answer. */
  private waiting = false;

  private hooks: AgentHooks;
  private activeStreamId: string | null = null;
  private activeAssistantId: string | null = null;
  private activeCallId: string | null = null;
  private cancelled = false;
  /** Anyone waiting on "has the user pressed Stop yet". */
  private stopWaiters: Array<() => void> = [];

  constructor(hooks: AgentHooks) {
    this.hooks = hooks;
  }

  /**
   * Where this chat stands, for the list and the status line. Derived rather
   * than stored, so it cannot drift from what the agent is really doing.
   */
  get status(): SessionStatus {
    if (this.running) return this.waiting ? "waiting" : "working";
    if (this.error) return "error";
    if (this.cancelled) return "cancelled";
    return this.items.length ? "completed" : "idle";
  }

  // ------------------------------------------------------------- state

  private update(id: string, patch: Partial<Item>) {
    const index = this.items.findIndex((i) => i.id === id);
    if (index === -1) return;
    this.items[index] = { ...this.items[index], ...patch } as Item;
    this.hooks.onChange();
  }

  private push(item: Item) {
    this.items.push(item);
    this.hooks.onChange();
    return item.id;
  }

  reset() {
    this.items = [];
    this.messages = [];
    this.error = null;
    this.cancelled = false;
    this.stopWaiters = [];
    this.hooks.onChange();
  }

  load(conversation: Conversation) {
    this.items = conversation.items ?? [];
    this.messages = conversation.messages ?? [];
    this.error = null;
    this.hooks.onChange();
  }

  /** Streaming text from the model, routed by stream id. */
  applyDelta(streamId: string, kind: string, text: string) {
    if (streamId !== this.activeStreamId || !this.activeAssistantId) return;
    const item = this.items.find((i) => i.id === this.activeAssistantId);
    if (!item || item.kind !== "assistant") return;
    if (kind === "reasoning") item.reasoning += text;
    else item.text += text;
    this.hooks.onChange();
  }

  /**
   * Where a long command has got to. One of these stands in for the hundreds of
   * redraws a renderer or an encoder would otherwise send, so the run shows
   * "55% · 207/375" instead of scrolling a bar past the user.
   */
  applyShellProgress(callId: string, progress: ToolProgress) {
    const item = this.items.find((i) => i.kind === "tool" && i.callId === callId);
    if (!item || item.kind !== "tool") return;
    item.progress = progress;
    this.hooks.onChange();
  }

  /**
   * Live stdout/stderr from a running command, in batches. A render prints a
   * line per frame; one repaint per line is what makes the window stutter
   * halfway through a long one.
   */
  applyShellOutput(callId: string, lines: string[]) {
    if (!lines.length) return;
    const item = this.items.find((i) => i.kind === "tool" && i.callId === callId);
    if (!item || item.kind !== "tool") return;
    item.output.push(...lines);
    if (item.output.length > 500) item.output.splice(0, item.output.length - 500);
    this.hooks.onChange();
  }

  /** Drop the last exchange so the same request can be run again. */
  rewindToLastUser() {
    if (this.running) return;
    const itemIndex = this.items.map((i) => i.kind).lastIndexOf("user");
    if (itemIndex === -1) return;
    this.items = this.items.slice(0, itemIndex);
    const messageIndex = this.messages.map((m) => m.role).lastIndexOf("user");
    if (messageIndex !== -1) this.messages = this.messages.slice(0, messageIndex);
    this.hooks.onChange();
  }

  cancel() {
    this.cancelled = true;
    if (this.activeStreamId) void api.cancelStream(this.activeStreamId);
    // Also stop whatever is executing, so Stop ends a long render rather than
    // just declining to start the next step.
    if (this.activeCallId) void api.cancelTool(this.activeCallId);
    // And release anything waiting on the user — an approval prompt nobody is
    // going to answer now would otherwise hold the run open indefinitely.
    this.stopWaiters.splice(0).forEach((wake) => wake());
    this.hooks.closeApproval();
    this.hooks.closeQuestion();
    this.hooks.onChange();
  }

  /** Resolves when Stop is pressed. Never resolves otherwise. */
  private stopped(): Promise<"stopped"> {
    if (this.cancelled) return Promise.resolve("stopped");
    return new Promise((resolve) => this.stopWaiters.push(() => resolve("stopped")));
  }

  // -------------------------------------------------------------- loop

  async send(text: string) {
    if (this.running) return;
    this.running = true;
    this.cancelled = false;
    this.stopWaiters = [];
    this.error = null;
    const turnStart = Date.now();

    this.push({ kind: "user", id: uid(), text });
    this.messages.push({ role: "user", content: text });

    try {
      await this.loop();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      if (message !== "cancelled") {
        this.error = message;
        this.push({ kind: "error", id: uid(), text: message });
      }
    } finally {
      await this.collectArtifacts(turnStart);
      this.running = false;
      this.activeStreamId = null;
      this.activeAssistantId = null;
      this.hooks.onChange();
    }
  }

  private async loop() {
    // Rebuilt every turn: it carries the live workspace, skills and mode.
    const systemPrompt = await api.getSystemPrompt();

    for (let step = 0; step < MAX_STEPS; step++) {
      if (this.cancelled) return;

      const streamId = uid();
      const assistantId = uid();
      this.activeStreamId = streamId;
      this.activeAssistantId = assistantId;
      this.push({
        kind: "assistant",
        id: assistantId,
        text: "",
        reasoning: "",
        streaming: true,
      });

      const conversation: ChatMessage[] = [
        { role: "system", content: systemPrompt },
        ...this.messages,
      ];

      let assistant;
      try {
        assistant = await api.chatStream(conversation, streamId);
      } catch (err) {
        this.items = this.items.filter((i) => i.id !== assistantId);
        throw err;
      } finally {
        this.activeStreamId = null;
      }

      this.update(assistantId, { streaming: false, text: assistant.content });
      if (!assistant.content.trim() && !assistant.reasoning.trim()) {
        this.items = this.items.filter((i) => i.id !== assistantId);
        this.hooks.onChange();
      }
      this.activeAssistantId = null;

      this.messages.push({
        role: "assistant",
        content: assistant.content,
        ...(assistant.tool_calls.length
          ? {
              tool_calls: assistant.tool_calls.map((c) => ({
                id: c.id,
                type: "function" as const,
                function: { name: c.name, arguments: c.arguments },
              })),
            }
          : {}),
      });

      if (!assistant.tool_calls.length) return;

      for (const call of assistant.tool_calls) {
        if (this.cancelled) {
          // The model is waiting on a result for every call it made, so answer
          // them all even when the user stopped the run.
          this.messages.push({
            role: "tool",
            tool_call_id: call.id,
            content: JSON.stringify({ ok: false, error: "The user stopped this run." }),
          });
          continue;
        }
        await this.runToolCall(call);
      }
    }

    this.push({
      kind: "error",
      id: uid(),
      text: `Stopped after ${MAX_STEPS} steps without finishing. Send another message to continue.`,
    });
  }

  /**
   * The agent asking the user something. It is a tool call like any other — the
   * model asks, an answer comes back — but the answer comes from the person
   * rather than from the machine, so it is handled here instead of in the
   * runtime. The run genuinely waits: the loop does not continue until there is
   * an answer or the user has stopped it.
   */
  private async askQuestion(call: ToolCall, args: Record<string, unknown>) {
    const rawOptions = Array.isArray(args.options) ? args.options : [];
    const question: Question = {
      question: String(args.question ?? "").trim() || "Which would you prefer?",
      context: typeof args.context === "string" ? args.context : undefined,
      options: rawOptions
        .map((o) => {
          const option = (o ?? {}) as Record<string, unknown>;
          return {
            label: String(option.label ?? "").trim(),
            detail: typeof option.detail === "string" ? option.detail : undefined,
          };
        })
        .filter((o) => o.label),
      allowOther: args.allow_other !== false,
    };
    // A question with nothing to pick from would be a dead end; leave a way to
    // answer it in words.
    if (!question.options.length) question.allowOther = true;

    const itemId = uid();
    this.push({
      kind: "tool",
      id: itemId,
      callId: call.id,
      name: call.name,
      title: "Question for you",
      detail: question.question,
      purpose: "",
      status: "awaiting",
      question,
      summary: "",
      output: [],
      resultText: "",
    });

    this.waiting = true;
    this.hooks.onChange();
    const answer = await Promise.race([
      this.hooks.askUser({ itemId, question }),
      this.stopped(),
    ]).finally(() => {
      this.waiting = false;
    });

    if (answer === "stopped" || answer === null) {
      this.update(itemId, { status: "cancelled", summary: "not answered" });
      this.messages.push({
        role: "tool",
        tool_call_id: call.id,
        content: JSON.stringify({
          ok: false,
          cancelled: true,
          error:
            "The user did not answer. Do not ask again — either carry on with a sensible default and say which you chose, or stop and explain what you need.",
        }),
      });
      return;
    }

    this.update(itemId, {
      status: "ok",
      summary: answer,
      question: { ...question, answer },
      resultText: `${question.question}\n\n${answer}`,
    });
    this.messages.push({
      role: "tool",
      tool_call_id: call.id,
      content: JSON.stringify({ ok: true, result: { answer } }),
    });
  }

  private async runToolCall(call: ToolCall) {
    let args: Record<string, unknown>;
    try {
      args = call.arguments.trim() ? JSON.parse(call.arguments) : {};
    } catch {
      this.messages.push({
        role: "tool",
        tool_call_id: call.id,
        content: JSON.stringify({
          ok: false,
          error: "Arguments were not valid JSON. Send the same call again with valid JSON.",
        }),
      });
      return;
    }

    if (call.name === "ask_user") {
      await this.askQuestion(call, args);
      return;
    }

    const evaluation = await api.evaluateTool(call.name, args);
    const itemId = uid();
    this.push({
      kind: "tool",
      id: itemId,
      callId: call.id,
      name: call.name,
      title: evaluation.title,
      detail: evaluation.detail || describeArgs(call.name, args),
      purpose: typeof args.purpose === "string" ? args.purpose : "",
      status: evaluation.decision === "ask" ? "awaiting" : "running",
      evaluation,
      summary: "",
      output: [],
      resultText: "",
    });

    let approved = evaluation.decision === "allow";
    if (evaluation.decision === "ask") {
      this.waiting = true;
      this.hooks.onChange();
      const answer = await Promise.race([
        this.hooks.requestApproval({ itemId, tool: call.name, args, evaluation }),
        this.stopped(),
      ]).finally(() => {
        this.waiting = false;
      });
      if (answer === "stopped") {
        this.update(itemId, { status: "cancelled", summary: "stopped" });
        this.messages.push({
          role: "tool",
          tool_call_id: call.id,
          content: JSON.stringify({
            ok: false,
            cancelled: true,
            error: "The user stopped this run before approving the action.",
          }),
        });
        return;
      }
      approved = answer;
      if (!approved) {
        this.update(itemId, { status: "denied", summary: "denied" });
      } else {
        this.update(itemId, { status: "running" });
      }
    }

    const startedAt = Date.now();
    this.activeCallId = call.id;
    let response;
    try {
      response = await api.runTool(call.name, args, call.id, approved);
    } finally {
      this.activeCallId = null;
    }
    const durationMs = Date.now() - startedAt;

    if (response.ok) {
      const result = response.result as Record<string, unknown>;
      // A command the user stopped is not a failure to diagnose, and it is not
      // a success either: it has an ending of its own.
      const stopped = response.cancelled === true || result.status === "cancelled";
      const failed = call.name === "shell" && result.exit_code !== 0;
      this.update(itemId, {
        status: stopped ? "cancelled" : failed ? "error" : "ok",
        summary: summarize(call.name, result),
        resultText: previewOf(call.name, result),
        durationMs,
      });
    } else {
      const denied = evaluation.decision === "ask" && !approved;
      const stopped = response.cancelled === true;
      this.update(itemId, {
        status: denied ? "denied" : stopped ? "cancelled" : "error",
        summary: denied ? "denied" : stopped ? "stopped" : (response.error ?? "failed"),
        resultText: response.error ?? "",
        durationMs,
      });
    }

    this.messages.push({
      role: "tool",
      tool_call_id: call.id,
      content: JSON.stringify(response),
    });
  }

  private async collectArtifacts(sinceMs: number) {
    try {
      const found: Artifact[] = await api.scanArtifacts(sinceMs);
      const shown = deliverables(found);
      if (shown.length) this.push({ kind: "artifacts", id: uid(), items: shown });
    } catch {
      // A missing or unreadable workspace is already reported elsewhere.
    }
  }
}

function describeArgs(tool: string, args: Record<string, unknown>): string {
  if (tool === "search_api_capabilities" || tool === "find_models")
    return String(args.query ?? args.produces ?? "");
  if (tool === "run_model") return String(args.model ?? "");
  if (tool === "configure_api") return String(args.api_id ?? "");
  if (tool === "speak") return String(args.text ?? "").slice(0, 90);
  if (tool === "analyze_reference") return String(args.url ?? "");
  if (tool === "see")
    return String(args.path ?? (Array.isArray(args.paths) ? (args.paths as string[]).join(", ") : ""));
  if (tool === "read_api_docs") return String(args.api_id ?? "");
  if (tool === "search_app_tools") return String(args.query ?? "");
  if (tool === "run_app_tool") return String(args.tool_slug ?? "");
  if (tool === "list_apis" || tool === "list_skills" || tool === "list_connected_apps") return "";
  if (typeof args.path === "string") return args.path;
  if (typeof args.name === "string") return args.name;
  return "";
}

const bytes = (n: number) => {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
};

/** m:ss, which is how a person reads a timestamp in a transcript. */
const clock = (seconds: number) => {
  const total = Math.max(0, Math.floor(seconds));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
};

const when = (ms: unknown) =>
  typeof ms === "number" && ms > 0 ? new Date(ms).toLocaleString() : "";

const count = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

const list = (r: Record<string, unknown>, key: string): Record<string, unknown>[] =>
  Array.isArray(r[key]) ? (r[key] as Record<string, unknown>[]) : [];

function summarize(tool: string, r: Record<string, unknown>): string {
  switch (tool) {
    case "shell": {
      const secs = ((r.duration_ms as number) / 1000).toFixed(1);
      if (r.status === "cancelled") return `stopped after ${secs}s`;
      if (r.timed_out) return "timed out";
      const code = r.exit_code;
      return code === 0 ? `${secs}s` : `failed · exit ${code}`;
    }
    case "fs_list": {
      const n = list(r, "entries").length;
      return `${count(n, "item")}${r.truncated ? " (partial)" : ""}`;
    }
    case "fs_read":
      return `${bytes((r.bytes as number) ?? 0)}${r.truncated ? " (partial)" : ""}`;
    case "fs_write":
      return `${r.created ? "created" : "updated"} · ${bytes((r.bytes_written as number) ?? 0)}`;
    case "fs_edit":
      return count((r.replacements as number) ?? 0, "replacement");
    case "fs_mkdir":
      return "created";
    case "fs_stat":
      return r.exists ? `exists · ${bytes((r.size as number) ?? 0)}` : "not found";
    case "list_skills":
      return count(list(r, "skills").length, "skill");
    case "read_skill":
      return "loaded";
    case "list_apis":
      return count(list(r, "connected").length, "API", "APIs");
    case "search_api_capabilities":
      return count(list(r, "matches").length, "match", "matches");
    case "read_api_docs":
      return "documentation";
    case "find_models":
      return count(list(r, "matches").length, "model");
    case "configure_api":
      return "set up";
    case "transcribe": {
      const secs = (r.duration_seconds as number) ?? 0;
      return secs ? `${Math.round(secs)}s of speech` : "transcribed";
    }
    case "speak":
      return `${bytes(((r.file as Record<string, unknown>)?.bytes as number) ?? 0)} of audio`;
    case "configure_api": {
      const changed = Array.isArray(r.changed) ? (r.changed as string[]) : [];
      return `${r.api_name}: ${changed.join(", ")}.`;
    }

    case "transcribe": {
      const utterances = list(r, "utterances");
      const files = list(r, "files");
      const head = [
        `${Math.round((r.duration_seconds as number) ?? 0)} seconds of speech, ${count(
          (r.utterance_count as number) ?? 0,
          "line",
        )}.`,
        files.length ? `Saved ${files.map((f) => f.path).join(" and ")}.` : "",
      ]
        .filter(Boolean)
        .join(" ");

      const body = utterances.length
        ? utterances
            .map((u) => {
              const at = clock((u.start as number) ?? 0);
              const who = u.speaker === undefined || u.speaker === null ? "" : ` Speaker ${u.speaker}:`;
              return `${at}${who} ${u.text}`;
            })
            .join("\n")
        : String(r.transcript ?? "");

      return `${head}\n\n${body}${r.truncated ? "\n\n… more lines are in the saved transcript." : ""}`;
    }

    case "speak": {
      const file = (r.file ?? {}) as Record<string, unknown>;
      return `Saved ${file.path} — ${bytes((file.bytes as number) ?? 0)}, read by ${r.voice}.`;
    }

    case "run_model": {
      const files = list(r, "files").length;
      return files ? count(files, "file") : "text only";
    }
    case "see": {
      const looked = list(r, "looked_at").length;
      return looked ? `looked at ${count(looked, "image")}` : "looked";
    }
    case "analyze_reference": {
      const confidence = typeof r.confidence === "number" ? Math.round(r.confidence * 100) : null;
      return `${r.scope}${confidence === null ? "" : ` · ${confidence}% sure`}`;
    }
    case "call_api": {
      const secs = ((r.duration_ms as number) / 1000).toFixed(1);
      return `${r.status} · ${secs}s`;
    }
    case "list_connected_apps":
      return count(list(r, "connected").length, "app");
    case "search_app_tools":
      return count(list(r, "matches").length, "action");
    case "run_app_tool":
      return String(r.app ?? "done");
    default:
      return "done";
  }
}

/**
 * What the card shows when you open it. Plain reading material — a person
 * should never have to parse JSON to see what the agent just did.
 */
function previewOf(tool: string, r: Record<string, unknown>): string {
  switch (tool) {
    case "shell": {
      const parts: string[] = [];
      if (typeof r.stdout === "string" && r.stdout.trim()) parts.push(r.stdout.trimEnd());
      if (typeof r.stderr === "string" && r.stderr.trim()) parts.push(r.stderr.trimEnd());
      if (typeof r.error === "string") parts.push(r.error);
      return parts.join("\n");
    }

    case "fs_read":
    case "read_skill":
      return String(r.content ?? "");

    case "fs_list": {
      const entries = list(r, "entries");
      if (!entries.length) return `${r.directory || "This folder"} is empty.`;
      const lines = entries.map((e) =>
        e.is_dir ? `${e.name}/` : `${e.name}  ·  ${bytes((e.size as number) ?? 0)}`,
      );
      if (r.truncated) lines.push("… more items not listed");
      return lines.join("\n");
    }

    case "fs_write":
      return `${r.created ? "Created" : "Updated"} ${r.path} — ${bytes(
        (r.bytes_written as number) ?? 0,
      )}.`;

    case "fs_edit":
      return `Edited ${r.path} — ${count((r.replacements as number) ?? 0, "replacement")} made, now ${bytes(
        (r.bytes as number) ?? 0,
      )}.`;

    case "fs_mkdir":
      return `Created the folder ${r.path}.`;

    case "fs_stat": {
      if (!r.exists) return `${r.path} does not exist.`;
      const modified = when(r.modified_ms);
      return [
        `${r.path} — ${r.is_dir ? "folder" : `file, ${bytes((r.size as number) ?? 0)}`}`,
        modified && `Last changed ${modified}`,
        `Full path: ${r.absolute_path}`,
      ]
        .filter(Boolean)
        .join("\n");
    }

    case "list_skills": {
      const skills = list(r, "skills");
      if (!skills.length) return "No skills are installed.";
      return skills
        .map((s) => [`${s.name}`, s.description && `  ${s.description}`].filter(Boolean).join("\n"))
        .join("\n\n");
    }

    case "list_apis": {
      const connected = list(r, "connected");
      if (!connected.length) return "No APIs are connected.";
      return connected
        .map((a) =>
          [
            `${a.name}`,
            a.notes && `  ${a.notes}`,
            `  ${
              (a.operations as number) > 0
                ? count(a.operations as number, "known operation")
                : `documentation ${a.documentation}`
            }`,
          ]
            .filter(Boolean)
            .join("\n"),
        )
        .join("\n\n");
    }

    case "search_api_capabilities": {
      const matches = list(r, "matches");
      const pending = list(r, "needs_documentation");
      const blocks: string[] = [];
      if (matches.length) {
        blocks.push(
          matches
            .map((m) =>
              [
                `${m.api_name} — ${m.operation}  (${m.method} ${m.path})`,
                m.description && `  ${m.description}`,
              ]
                .filter(Boolean)
                .join("\n"),
            )
            .join("\n\n"),
        );
      } else {
        blocks.push("Nothing matched.");
      }
      if (pending.length) {
        blocks.push(pending.map((p) => `${p.name}: ${p.hint}`).join("\n"));
      }
      return blocks.join("\n\n");
    }

    case "read_api_docs":
      return [`Documentation for ${r.api_name}`, "", String(r.documentation ?? "")].join("\n");

    case "find_models": {
      const matches = list(r, "matches");
      if (!matches.length) return String(r.note ?? "Nothing matched.");
      return matches
        .map((m) =>
          [
            `${m.name || m.model}`,
            `  ${m.model}`,
            `  produces ${(m.produces as string[]).join(", ")} · ${
              m.price_per_million_input_tokens
                ? `${m.price_per_million_input_tokens} per million input tokens`
                : "price unlisted"
            }`,
          ].join("\n"),
        )
        .join("\n\n");
    }

    case "configure_api": {
      const changed = Array.isArray(r.changed) ? (r.changed as string[]) : [];
      return `${r.api_name}: ${changed.join(", ")}.`;
    }

    case "transcribe": {
      const utterances = list(r, "utterances");
      const files = list(r, "files");
      const head = [
        `${Math.round((r.duration_seconds as number) ?? 0)} seconds of speech, ${count(
          (r.utterance_count as number) ?? 0,
          "line",
        )}.`,
        files.length ? `Saved ${files.map((f) => f.path).join(" and ")}.` : "",
      ]
        .filter(Boolean)
        .join(" ");

      const body = utterances.length
        ? utterances
            .map((u) => {
              const at = clock((u.start as number) ?? 0);
              const who = u.speaker === undefined || u.speaker === null ? "" : ` Speaker ${u.speaker}:`;
              return `${at}${who} ${u.text}`;
            })
            .join("\n")
        : String(r.transcript ?? "");

      return `${head}\n\n${body}${r.truncated ? "\n\n… more lines are in the saved transcript." : ""}`;
    }

    case "speak": {
      const file = (r.file ?? {}) as Record<string, unknown>;
      return `Saved ${file.path} — ${bytes((file.bytes as number) ?? 0)}, read by ${r.voice}.`;
    }

    case "run_model": {
      const files = list(r, "files");
      const lines: string[] = [];
      if (files.length) {
        lines.push(
          files
            .map((f) => `Saved ${f.path} — ${f.kind}, ${bytes((f.bytes as number) ?? 0)}`)
            .join("\n"),
        );
      }
      if (typeof r.text === "string" && r.text.trim()) lines.push(r.text.trim());
      return lines.join("\n\n") || `${r.model} returned nothing.`;
    }

    case "analyze_reference": {
      const analysis = (r.analysis ?? {}) as Record<string, unknown>;
      const scope = String(r.scope ?? "full");
      const block = (analysis[scope] ?? analysis) as Record<string, unknown>;
      const notes = block.rebuildNotes ?? analysis.rebuildNotes;
      return [
        `${r.model} watched ${r.url} — nothing downloaded.`,
        typeof notes === "string" ? notes : "",
        `Saved to ${r.saved}`,
        JSON.stringify(analysis, null, 2),
      ]
        .filter(Boolean)
        .join("\n\n");
    }

    case "see": {
      const looked = list(r, "looked_at")
        .map((l) =>
          l.frame_at_seconds === undefined
            ? String(l.path)
            : `${l.path} at ${clock(l.frame_at_seconds as number)}`,
        )
        .join(", ");
      const answer = String(r.answer ?? "").trim();
      return looked ? `${looked}\n\n${answer}` : answer;
    }

    case "call_api": {
      const secs = ((r.duration_ms as number) / 1000).toFixed(1);
      const header = `${r.api} answered ${r.status} in ${secs}s · ${bytes(
        (r.bytes as number) ?? 0,
      )}${r.truncated ? " (partial)" : ""}`;
      const body = readable(r.body).join("\n");
      return body ? `${header}\n\n${body}` : header;
    }

    case "list_connected_apps": {
      const connected = list(r, "connected");
      if (!connected.length) return String(r.note ?? "No apps are connected.");
      return connected
        .map((a) => `${a.name}${a.ready ? "" : `  (${String(a.status).toLowerCase()})`}`)
        .join("\n");
    }

    case "search_app_tools": {
      const matches = list(r, "matches");
      if (!matches.length) return String(r.note ?? "Nothing matched.");
      return matches
        .map((m) =>
          [`${m.name || m.tool_slug}`, `  ${m.tool_slug}`, m.description && `  ${m.description}`]
            .filter(Boolean)
            .join("\n"),
        )
        .join("\n\n");
    }

    case "run_app_tool": {
      const header = `${r.app} — ${r.tool_slug}`;
      const body = readable(r.data).join("\n");
      return body ? `${header}\n\n${body}` : header;
    }

    default:
      return readable(r).join("\n");
  }
}

/**
 * Turn any result into an indented outline a person can read. Structure is
 * conveyed by indentation rather than braces and quotes, and long or deep data
 * is cut off rather than dumped.
 */
function readable(value: unknown, depth = 0): string[] {
  const pad = "  ".repeat(depth);
  if (value === null || value === undefined) return [`${pad}—`];
  if (typeof value === "string") return value.split("\n").map((l) => `${pad}${l}`);
  if (typeof value !== "object") return [`${pad}${String(value)}`];

  if (depth >= 4) return [`${pad}…`];

  if (Array.isArray(value)) {
    if (!value.length) return [`${pad}(none)`];
    const lines: string[] = [];
    value.slice(0, 20).forEach((item, i) => {
      if (item === null || typeof item !== "object") {
        lines.push(`${pad}• ${String(item ?? "—")}`);
      } else {
        lines.push(`${pad}• Item ${i + 1}`);
        lines.push(...readable(item, depth + 1));
      }
    });
    if (value.length > 20) lines.push(`${pad}… ${value.length - 20} more`);
    return lines;
  }

  const entries = Object.entries(value as Record<string, unknown>);
  if (!entries.length) return [`${pad}(empty)`];
  const lines: string[] = [];
  for (const [key, entry] of entries.slice(0, 40)) {
    const label = key.replace(/_/g, " ");
    if (entry === null || typeof entry !== "object") {
      lines.push(`${pad}${label}: ${entry === null ? "—" : String(entry)}`);
    } else if (Array.isArray(entry) && entry.every((e) => e === null || typeof e !== "object")) {
      lines.push(`${pad}${label}: ${entry.length ? entry.join(", ") : "(none)"}`);
    } else {
      lines.push(`${pad}${label}:`);
      lines.push(...readable(entry, depth + 1));
    }
  }
  if (entries.length > 40) lines.push(`${pad}… ${entries.length - 40} more fields`);
  return lines;
}
