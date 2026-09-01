import { useState } from "react";
import { Avatar, Button } from "@heroui/react";
import type { SessionStatus } from "../lib/sessions";
import logoUrl from "../assets/logo.png";
import {
  AppsIcon,
  ChatIcon,
  FolderOpenIcon,
  PanelIcon,
  PlugIcon,
  PlusSquareIcon,
  ProjectsIcon,
  SkillsIcon,
  PencilIcon,
  TrashIcon,
} from "./Icons";

/** A chat in the list: a saved one, or one with an agent working inside it. */
export interface ChatRow {
  id: string;
  title: string;
  status: SessionStatus;
  unread: boolean;
  open: boolean;
}

export function Sidebar({
  chats,
  currentId,
  workspace,
  collapsed,
  onToggle,
  onNew,
  onOpen,
  onDelete,
  onRename,
  onSkills,
  onWorkspaces,
  onApis,
  onApps,
  onChooseFolder,
}: {
  chats: ChatRow[];
  currentId: string;
  workspace: string | null;
  collapsed: boolean;
  onToggle: () => void;
  onNew: () => void;
  onOpen: (id: string) => void;
  onDelete: (id: string) => void;
  onRename: (id: string, title: string) => void;
  onSkills: () => void;
  onWorkspaces: () => void;
  onApis: () => void;
  onApps: () => void;
  onChooseFolder: () => void;
}) {
  if (collapsed) return null;

  const project = workspace ? workspace.split("/").filter(Boolean).pop() : null;

  return (
    <aside className="flex h-full w-[264px] shrink-0 flex-col border-r border-border bg-background-secondary">
      <div className="flex items-center gap-3 px-4 pb-3 pt-4">
        <Avatar size="sm" className="bg-[#141210]">
          <Avatar.Image src={logoUrl} alt="SirVibe" className="scale-[0.72]" />
          <Avatar.Fallback>SV</Avatar.Fallback>
        </Avatar>
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold text-foreground">SirVibe</div>
        </div>
        <button
          onClick={onToggle}
          title="Hide sidebar"
          className="rounded-lg p-1.5 text-muted transition-colors hover:bg-default hover:text-foreground"
        >
          <PanelIcon />
        </button>
      </div>

      <nav className="flex flex-col gap-0.5 px-2">
        <NavItem icon={<PlusSquareIcon />} label="New Chat" onClick={onNew} />
      </nav>

      {/* The working folder, one click from anywhere. */}
      <button
        onClick={onChooseFolder}
        title={workspace ?? "Choose a folder for the agent to work in"}
        className="mx-2 mt-1.5 flex items-center gap-2.5 rounded-lg border border-border px-2.5 py-2 text-left transition-colors hover:border-accent hover:bg-default/60"
      >
        <FolderOpenIcon className="h-4 w-4 shrink-0 text-muted" />
        <span className="min-w-0 flex-1">
          <span className="block text-[10px] font-medium tracking-wide text-muted uppercase">
            Workspace
          </span>
          <span className="block truncate text-[13px] font-medium text-foreground">
            {project ?? "Choose a folder"}
          </span>
        </span>
      </button>

      <nav className="mt-2 flex flex-col gap-0.5 px-2">
        <NavItem icon={<SkillsIcon />} label="Skills" onClick={onSkills} />
        <NavItem icon={<ProjectsIcon />} label="Workspaces" onClick={onWorkspaces} />
        <NavItem icon={<PlugIcon />} label="APIs" onClick={onApis} />
        <NavItem icon={<AppsIcon />} label="Apps" onClick={onApps} />
      </nav>

      <div className="px-4 pb-1 pt-5 text-xs font-medium text-muted">Chats</div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {chats.length === 0 && (
          <div className="px-2 py-1.5 text-sm text-muted">No chats yet.</div>
        )}
        {chats.map((chat) => (
          <ChatRowItem
            key={chat.id}
            chat={chat}
            active={chat.id === currentId}
            onOpen={onOpen}
            onDelete={onDelete}
            onRename={onRename}
          />
        ))}
      </div>
    </aside>
  );
}

/**
 * One chat. Shows what it is called and what it is doing — a chat working out
 * of sight is the whole point of the list.
 */
function ChatRowItem({
  chat,
  active,
  onOpen,
  onDelete,
  onRename,
}: {
  chat: ChatRow;
  active: boolean;
  onOpen: (id: string) => void;
  onDelete: (id: string) => void;
  onRename: (id: string, title: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(chat.title);

  const commit = () => {
    const clean = draft.trim();
    if (clean && clean !== chat.title) onRename(chat.id, clean);
    setEditing(false);
  };

  if (editing) {
    return (
      <form
        className="px-2 py-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          commit();
        }}
      >
        <input
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setDraft(chat.title);
              setEditing(false);
            }
          }}
          className="w-full rounded-lg border border-accent bg-field px-2 py-1.5 text-sm outline-none"
        />
      </form>
    );
  }

  return (
    <div
      className={`group flex items-center gap-2 rounded-lg pr-1 transition-colors ${
        active ? "bg-default" : "hover:bg-default/60"
      }`}
    >
      <button
        onClick={() => onOpen(chat.id)}
        onDoubleClick={() => {
          setDraft(chat.title);
          setEditing(true);
        }}
        title={chat.title}
        className="flex min-w-0 flex-1 items-center gap-2.5 px-2 py-2 text-left text-sm text-foreground"
      >
        <ChatMark status={chat.status} />
        <span className="min-w-0 flex-1 truncate">
          {chat.title || "Untitled"}
          {chat.status === "working" && (
            <span className="block text-[11px] text-muted">Working…</span>
          )}
          {chat.status === "waiting" && (
            <span className="block text-[11px] text-warning-foreground">Needs you</span>
          )}
        </span>
        {chat.unread && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-success" />}
      </button>
      <button
        onClick={() => {
          setDraft(chat.title);
          setEditing(true);
        }}
        title="Rename"
        className="rounded-md p-1.5 text-muted opacity-0 transition hover:text-foreground focus:opacity-100 group-hover:opacity-100"
      >
        <PencilIcon className="h-3.5 w-3.5" />
      </button>
      <button
        onClick={() => onDelete(chat.id)}
        title="Delete chat"
        className="rounded-md p-1.5 text-muted opacity-0 transition hover:text-danger focus:opacity-100 group-hover:opacity-100"
      >
        <TrashIcon className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

/** What this chat is doing, as one small mark. */
function ChatMark({ status }: { status: SessionStatus }) {
  if (status === "working") {
    return (
      <span className="relative flex h-4 w-4 shrink-0 items-center justify-center">
        <span className="absolute h-2 w-2 animate-ping rounded-full bg-success opacity-75" />
        <span className="h-2 w-2 rounded-full bg-success" />
      </span>
    );
  }
  if (status === "waiting")
    return <span className="h-4 w-4 shrink-0 text-center text-[13px] text-warning">?</span>;
  if (status === "completed")
    return <span className="h-4 w-4 shrink-0 text-center text-[13px] text-success">✓</span>;
  if (status === "error")
    return <span className="h-4 w-4 shrink-0 text-center text-[13px] text-danger">✕</span>;
  if (status === "cancelled")
    return <span className="h-4 w-4 shrink-0 text-center text-[13px] text-muted">⊘</span>;
  return <ChatIcon className="h-4 w-4 shrink-0 text-muted" />;
}

function NavItem({
  icon,
  label,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <Button
      variant="ghost"
      onPress={onClick}
      className="h-9 w-full justify-start gap-2.5 px-2 text-sm font-normal text-foreground"
    >
      <span className="text-muted">{icon}</span>
      {label}
    </Button>
  );
}
