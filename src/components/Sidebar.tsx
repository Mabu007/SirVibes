import { Avatar, Button } from "@heroui/react";
import type { ConversationSummary } from "../lib/types";
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
  TrashIcon,
} from "./Icons";

export function Sidebar({
  history,
  currentId,
  workspace,
  collapsed,
  onToggle,
  onNew,
  onOpen,
  onDelete,
  onSkills,
  onWorkspaces,
  onApis,
  onApps,
  onChooseFolder,
}: {
  history: ConversationSummary[];
  currentId: string;
  workspace: string | null;
  collapsed: boolean;
  onToggle: () => void;
  onNew: () => void;
  onOpen: (id: string) => void;
  onDelete: (id: string) => void;
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

      <div className="px-4 pb-1 pt-5 text-xs font-medium text-muted">Recent</div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {history.length === 0 && (
          <div className="px-2 py-1.5 text-sm text-muted">No conversations yet.</div>
        )}
        {history.map((h) => {
          const active = h.id === currentId;
          return (
            <div
              key={h.id}
              className={`group flex items-center gap-2 rounded-lg pr-1 transition-colors ${
                active ? "bg-default" : "hover:bg-default/60"
              }`}
            >
              <button
                onClick={() => onOpen(h.id)}
                title={h.title}
                className="flex min-w-0 flex-1 items-center gap-2.5 px-2 py-2 text-left text-sm text-foreground"
              >
                <ChatIcon className="h-4 w-4 shrink-0 text-muted" />
                <span className="truncate">{h.title || "Untitled"}</span>
              </button>
              <button
                onClick={() => onDelete(h.id)}
                title="Delete conversation"
                className="rounded-md p-1.5 text-muted opacity-0 transition hover:text-danger focus:opacity-100 group-hover:opacity-100"
              >
                <TrashIcon className="h-3.5 w-3.5" />
              </button>
            </div>
          );
        })}
      </div>
    </aside>
  );
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
