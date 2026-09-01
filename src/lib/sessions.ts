import type { Agent } from "./agent";

/**
 * What a chat looks like from the outside: enough to draw it in the list and
 * to know whether it needs attention, without reaching into the agent running
 * inside it.
 */
export type SessionStatus =
  | "idle"
  | "working"
  | "waiting"
  | "completed"
  | "error"
  | "cancelled";

export interface SessionMeta {
  id: string;
  title: string;
  /** The user named it themselves, so nothing may rename it again. */
  titleLocked: boolean;
  status: SessionStatus;
  updatedMs: number;
  /** Finished while the user was looking at something else. */
  unread: boolean;
}

/**
 * Every chat that is open, and the agent working inside each one.
 *
 * The point of this class is that there is no "the agent" any more. A chat owns
 * its own agent, its own history and its own execution; opening a second chat
 * starts a second agent beside the first rather than displacing it. Nothing
 * here schedules or serialises anything — the agents are ordinary objects
 * running their own loops, and the runtime underneath already keys every model
 * stream and every tool call by its own id.
 */
export class SessionStore {
  private agents = new Map<string, Agent>();
  private metas = new Map<string, SessionMeta>();

  constructor(
    private readonly make: (id: string) => Agent,
    private readonly changed: () => void,
  ) {}

  /** The chat with this id, started if it is not open yet. */
  ensure(id: string, title = "New chat", titleLocked = false): Agent {
    const existing = this.agents.get(id);
    if (existing) return existing;

    const agent = this.make(id);
    this.agents.set(id, agent);
    this.metas.set(id, {
      id,
      title,
      titleLocked,
      status: "idle",
      updatedMs: Date.now(),
      unread: false,
    });
    return agent;
  }

  agent(id: string): Agent | undefined {
    return this.agents.get(id);
  }

  meta(id: string): SessionMeta | undefined {
    return this.metas.get(id);
  }

  /** Most recently active first, which is the order a person looks for them in. */
  list(): SessionMeta[] {
    return [...this.metas.values()].sort((a, b) => b.updatedMs - a.updatedMs);
  }

  open(): string[] {
    return [...this.agents.keys()];
  }

  /**
   * Re-read one chat's status from the agent inside it. Called whenever that
   * agent reports a change, which is what makes the list live.
   */
  refresh(id: string, visible: boolean) {
    const meta = this.metas.get(id);
    const agent = this.agents.get(id);
    if (!meta || !agent) return;

    const before = meta.status;
    meta.status = agent.status;
    meta.updatedMs = Date.now();

    // Something that finished out of sight is worth a mark; something the user
    // is already looking at is not.
    if (before === "working" || before === "waiting") {
      if (meta.status === "completed" || meta.status === "error") {
        meta.unread = !visible;
      }
    }
    if (visible) meta.unread = false;
    this.changed();
  }

  read(id: string) {
    const meta = this.metas.get(id);
    if (meta?.unread) {
      meta.unread = false;
      this.changed();
    }
  }

  rename(id: string, title: string, locked = true) {
    const meta = this.metas.get(id);
    if (!meta) return;
    const clean = title.trim();
    if (!clean) return;
    meta.title = clean.slice(0, 80);
    meta.titleLocked = locked;
    this.changed();
  }

  /** A generated title never overwrites one the user chose. */
  suggestTitle(id: string, title: string) {
    const meta = this.metas.get(id);
    if (!meta || meta.titleLocked) return;
    this.rename(id, title, false);
  }

  close(id: string) {
    this.agents.get(id)?.cancel();
    this.agents.delete(id);
    this.metas.delete(id);
    this.changed();
  }

  /** How many agents are actually doing something right now. */
  activeCount(): number {
    return [...this.agents.values()].filter(
      (a) => a.status === "working" || a.status === "waiting",
    ).length;
  }
}
