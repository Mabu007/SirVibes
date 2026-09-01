import { useEffect, useRef } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Button } from "@heroui/react";
import type { PermissionMode } from "../lib/types";
import { ArrowUpIcon, ChevronDownIcon, GlobeIcon, PaperclipIcon, StopIcon } from "./Icons";

const MODE_LABEL: Record<PermissionMode, string> = {
  ask: "Ask every time",
  smart: "Smart",
  full: "Full autonomy",
};

export function Composer({
  value,
  onChange,
  onSend,
  onStop,
  running,
  model,
  onPickModel,
  needsSetup,
  onNeedsSetup,
  mode,
  onMode,
}: {
  value: string;
  onChange: (text: string) => void;
  onSend: (text: string) => void;
  onStop: () => void;
  running: boolean;
  model: string;
  onPickModel: () => void;
  needsSetup: boolean;
  onNeedsSetup: () => void;
  mode: PermissionMode;
  onMode: (mode: PermissionMode) => void;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [value]);

  const submit = () => {
    const text = value.trim();
    if (!text || running) return;
    // Setup is missing, so open it rather than swallowing what they typed.
    if (needsSetup) {
      onNeedsSetup();
      return;
    }
    onChange("");
    onSend(text);
  };

  /**
   * Attach puts the file's full path into the message. Paths outside the
   * workspace are allowed — the agent still has to ask before touching them.
   */
  const attach = async () => {
    const picked = await open({ multiple: true, title: "Reference files" });
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    if (!paths.length) return;
    const block = paths.join("\n");
    onChange(value.trim() ? `${value.replace(/\s*$/, "")}\n${block}\n` : `${block}\n`);
    ref.current?.focus();
  };

  return (
    <div className="px-4 pb-3">
      <div className="mx-auto w-full max-w-3xl">
        <div className="rounded-2xl border border-border bg-background shadow-sm transition-shadow focus-within:shadow-md">
          <textarea
            ref={ref}
            rows={1}
            autoFocus
            placeholder="What do you want to make?"
            value={value}
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                submit();
              }
            }}
            className="w-full resize-none bg-transparent px-4 pt-3.5 pb-1 text-[15px] text-foreground outline-none placeholder:text-field-placeholder"
          />

          <div className="flex items-center gap-2 px-2.5 pb-2.5">
            <Button variant="ghost" size="sm" isIconOnly aria-label="Reference files" onPress={attach}>
              <PaperclipIcon />
            </Button>
            <Button variant="ghost" size="sm" onPress={onPickModel} className="gap-1.5 font-normal">
              <GlobeIcon className="h-3.5 w-3.5" />
              <span className="max-w-[220px] truncate text-[13px]">{model || "Select model"}</span>
              <ChevronDownIcon className="h-3.5 w-3.5 text-muted" />
            </Button>

            <div className="flex-1" />

            {/* How much rope the agent has, next to the thing you are about to
                ask it — not filed away in the window chrome. */}
            <select
              value={mode}
              onChange={(e) => onMode(e.target.value as PermissionMode)}
              aria-label="How much the agent may do on its own"
              className={`rounded-lg border px-2 py-1 text-[12.5px] outline-none ${
                mode === "full"
                  ? "border-warning/50 bg-warning/10 text-warning-foreground"
                  : "border-border bg-background text-muted"
              }`}
            >
              {(Object.keys(MODE_LABEL) as PermissionMode[]).map((m) => (
                <option key={m} value={m}>
                  {MODE_LABEL[m]}
                </option>
              ))}
            </select>

            {running ? (
              <Button variant="danger" size="sm" isIconOnly aria-label="Stop" onPress={onStop}>
                <StopIcon />
              </Button>
            ) : (
              <Button
                variant="primary"
                size="sm"
                isIconOnly
                aria-label="Send"
                isDisabled={!value.trim()}
                onPress={submit}
                className="rounded-full"
              >
                <ArrowUpIcon />
              </Button>
            )}
          </div>
        </div>

        {needsSetup ? (
          <button
            onClick={onNeedsSetup}
            className="mx-auto mt-2 block text-center text-xs text-warning-foreground underline decoration-dotted underline-offset-4"
          >
            Finish setup to run — API key, model and workspace
          </button>
        ) : (
          <p className="mt-2 text-center text-xs text-muted">
            The agent runs real commands on this computer. Check important results.
          </p>
        )}
      </div>
    </div>
  );
}
