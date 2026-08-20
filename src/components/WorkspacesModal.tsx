import { open } from "@tauri-apps/plugin-dialog";
import { Button } from "@heroui/react";
import { api } from "../lib/api";
import type { SettingsView } from "../lib/types";
import { Overlay } from "./Overlay";
import { CheckIcon, ProjectsIcon } from "./Icons";

/** A workspace is the folder the agent reads, writes and runs commands in. */
export function WorkspacesModal({
  settings,
  onSettings,
  onClose,
}: {
  settings: SettingsView;
  onSettings: (s: SettingsView) => void;
  onClose: () => void;
}) {
  const choose = async () => {
    const picked = await open({ directory: true, multiple: false, title: "Open workspace folder" });
    if (typeof picked === "string") {
      onSettings(await api.updateSettings({ workspace: picked }));
      onClose();
    }
  };

  const switchTo = async (path: string) => {
    onSettings(await api.updateSettings({ workspace: path }));
    onClose();
  };

  const recents = Array.from(
    new Set([settings.workspace, ...(settings.recent_workspaces ?? [])].filter(Boolean) as string[]),
  );

  return (
    <Overlay
      title="Workspaces"
      subtitle="A workspace is the folder the agent works in. It reads, writes and runs commands here — and nowhere else without asking."
      onClose={onClose}
      width="max-w-xl"
      footer={
        <Button variant="primary" onPress={choose}>
          Open folder…
        </Button>
      }
    >
      <div className="flex flex-col gap-1.5">
        {recents.length === 0 && (
          <p className="py-6 text-center text-sm text-muted">
            No workspaces yet. Open a folder to get started.
          </p>
        )}
        {recents.map((path) => {
          const active = path === settings.workspace;
          return (
            <button
              key={path}
              onClick={() => switchTo(path)}
              className={`flex items-center gap-3 rounded-xl border px-3.5 py-3 text-left transition-colors ${
                active ? "border-accent bg-accent/[0.06]" : "border-border hover:bg-default/60"
              }`}
            >
              <ProjectsIcon className="h-4 w-4 shrink-0 text-muted" />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium text-foreground">
                  {path.split("/").filter(Boolean).pop()}
                </span>
                <span className="block truncate font-mono text-[11.5px] text-muted">{path}</span>
              </span>
              {active && <CheckIcon className="h-4 w-4 shrink-0 text-accent" />}
            </button>
          );
        })}
      </div>
    </Overlay>
  );
}
