import { useState } from "react";
import { Button, Spinner } from "@heroui/react";
import type { Evaluation, Item, Question } from "../lib/types";
import { AlertIcon, ChevronDownIcon, TerminalIcon } from "./Icons";

type ToolItem = Extract<Item, { kind: "tool" }>;

/** Tools whose output is code or terminal text and must be shown verbatim. */
const VERBATIM = ["shell", "fs_read", "read_skill"];
/** Tools whose one-line detail is a path or a command. */
const PATHLIKE = [
  "shell",
  "fs_read",
  "fs_list",
  "fs_stat",
  "fs_write",
  "fs_edit",
  "fs_mkdir",
  "transcribe",
  "see",
];

export function ToolCard({
  item,
  awaitingHere,
  onDecide,
  askingHere,
  onAnswer,
}: {
  item: ToolItem;
  awaitingHere: boolean;
  onDecide: (approved: boolean) => void;
  askingHere?: boolean;
  onAnswer?: (answer: string | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const live = item.status === "running" && item.output.length > 0;
  const body = live ? item.output.join("\n") : item.resultText;
  const expandable = Boolean(body.trim());
  // Monospace is for things that are literally code or terminal output.
  // Everything else reads as ordinary text.
  const mono = live || VERBATIM.includes(item.name);

  // A question is asked in the conversation too, for the same reason: the work
  // that led to it is right there above it.
  if (askingHere && item.question && onAnswer) {
    return <QuestionCard question={item.question} onAnswer={onAnswer} />;
  }

  // Approval is asked for in the conversation, where you can see what led to it.
  if (awaitingHere && item.evaluation) {
    return <ApprovalCard item={item} evaluation={item.evaluation} onDecide={onDecide} />;
  }

  return (
    <div className="my-2 overflow-hidden rounded-xl border border-border bg-background">
      <button
        onClick={() => expandable && setOpen(!open)}
        disabled={!expandable}
        className="flex w-full items-center gap-2.5 px-3 py-2.5 text-left disabled:cursor-default"
      >
        <StatusDot status={item.status} />
        <span className="shrink-0 text-[13px] font-medium text-foreground">
          {item.purpose || item.title}
        </span>
        <span
          className={`min-w-0 flex-1 truncate text-xs text-muted ${
            PATHLIKE.includes(item.name) && !live ? "font-mono" : ""
          }`}
        >
          {/* While something long is running, where it has got to matters more
              than the command that started it. */}
          {item.status === "running" && item.progress ? item.progress.summary : item.detail}
        </span>
        <span className="shrink-0 font-mono text-[11px] tabular-nums text-muted">
          {item.summary}
        </span>
        {expandable && (
          <ChevronDownIcon
            className={`h-4 w-4 shrink-0 text-muted transition-transform ${open ? "rotate-180" : ""}`}
          />
        )}
      </button>

      {(open || live) && expandable && (
        <pre
          className={`max-h-[320px] overflow-auto border-t border-border bg-background-secondary px-3 py-2.5 leading-relaxed whitespace-pre-wrap break-words text-foreground/80 ${
            mono ? "font-mono text-[11.5px]" : "text-[12.5px]"
          }`}
        >
          {trim(body)}
        </pre>
      )}
    </div>
  );
}

function ApprovalCard({
  item,
  evaluation,
  onDecide,
}: {
  item: ToolItem;
  evaluation: Evaluation;
  onDecide: (approved: boolean) => void;
}) {
  return (
    <div className="my-3 overflow-hidden rounded-xl border border-warning/60 bg-warning/[0.06]">
      <div className="flex items-center gap-2 px-3 pt-3">
        <AlertIcon className="h-4 w-4 text-warning" />
        <span className="text-[13px] text-foreground">
          Approval needed: <span className="font-medium">{evaluation.title || item.name}</span>
        </span>
      </div>

      <pre
        className={`mx-3 mt-2.5 max-h-40 overflow-auto rounded-lg bg-background px-3 py-2.5 whitespace-pre-wrap break-words text-foreground/85 ${
          VERBATIM.includes(item.name) || PATHLIKE.includes(item.name)
            ? "font-mono text-[11.5px]"
            : "text-[12.5px]"
        }`}
      >
        {evaluation.detail}
      </pre>

      {evaluation.risks.length > 0 && (
        <ul className="mx-3 mt-2.5 flex flex-col gap-1.5">
          {evaluation.risks.map((r, i) => (
            <li
              key={i}
              className={`rounded-lg px-2.5 py-1.5 text-xs ${
                ["outside_workspace", "destructive", "privilege", "remote_exec", "external_side_effect"].includes(
                  r.kind,
                )
                  ? "bg-danger/10 text-danger"
                  : "bg-warning/10 text-warning-foreground"
              }`}
            >
              {r.message}
            </li>
          ))}
        </ul>
      )}

      <div className="flex justify-end gap-2 px-3 py-3">
        <Button size="sm" variant="secondary" onPress={() => onDecide(false)}>
          Reject
        </Button>
        <Button size="sm" variant="primary" onPress={() => onDecide(true)}>
          Approve
        </Button>
      </div>
    </div>
  );
}

/**
 * The agent stopped to ask something. Deliberately plain: the choices are
 * outcomes a person can picture, not settings, and answering is one click.
 */
function QuestionCard({
  question,
  onAnswer,
}: {
  question: Question;
  onAnswer: (answer: string | null) => void;
}) {
  const [other, setOther] = useState("");

  return (
    <div className="my-3 overflow-hidden rounded-xl border border-accent/50 bg-accent/[0.05]">
      {question.context && (
        <p className="px-3.5 pt-3 text-xs text-muted">{question.context}</p>
      )}
      <p className="px-3.5 pt-3 pb-1 text-[14px] font-medium text-foreground">
        {question.question}
      </p>

      <div className="flex flex-col gap-1.5 px-3.5 pt-1.5">
        {question.options.map((option) => (
          <button
            key={option.label}
            onClick={() => onAnswer(option.label)}
            className="rounded-xl border border-border bg-background px-3.5 py-2.5 text-left transition-colors hover:border-accent hover:bg-accent/[0.06]"
          >
            <span className="block text-[13.5px] font-medium text-foreground">{option.label}</span>
            {option.detail && (
              <span className="mt-0.5 block text-xs leading-snug text-muted">{option.detail}</span>
            )}
          </button>
        ))}
      </div>

      {question.allowOther && (
        <form
          className="flex gap-2 px-3.5 pt-2.5"
          onSubmit={(e) => {
            e.preventDefault();
            if (other.trim()) onAnswer(other.trim());
          }}
        >
          <input
            value={other}
            onChange={(e) => setOther(e.target.value)}
            placeholder="Or say what you would prefer"
            className="min-w-0 flex-1 rounded-lg border border-field-border bg-field px-3 py-1.5 text-[13px] outline-none focus:border-accent"
          />
          <Button size="sm" variant="secondary" type="submit" isDisabled={!other.trim()}>
            Send
          </Button>
        </form>
      )}

      <div className="flex justify-end px-3.5 py-3">
        <button
          onClick={() => onAnswer(null)}
          className="text-xs text-muted underline-offset-2 hover:underline"
        >
          Skip — you decide
        </button>
      </div>
    </div>
  );
}

function StatusDot({ status }: { status: ToolItem["status"] }) {
  if (status === "running") return <Spinner className="h-3.5 w-3.5 shrink-0 text-muted" />;
  const map: Record<string, string> = {
    ok: "text-success",
    error: "text-danger",
    denied: "text-warning",
    cancelled: "text-muted",
    awaiting: "text-warning",
  };
  const glyph: Record<string, string> = {
    ok: "✓",
    error: "✕",
    denied: "⊘",
    cancelled: "⊘",
    awaiting: "?",
  };
  if (status === "ok" || status === "error")
    return (
      <span className={`w-4 shrink-0 text-center text-[13px] ${map[status]}`}>{glyph[status]}</span>
    );
  return (
    <span className={`w-4 shrink-0 text-center text-[13px] ${map[status] ?? "text-muted"}`}>
      {glyph[status] ?? <TerminalIcon />}
    </span>
  );
}

/** Keep the tail: the end of a command's output is where the answer is. */
function trim(text: string): string {
  const lines = text.split("\n");
  if (lines.length <= 200) return text;
  return [`… ${lines.length - 200} earlier lines hidden …`, ...lines.slice(-200)].join("\n");
}
