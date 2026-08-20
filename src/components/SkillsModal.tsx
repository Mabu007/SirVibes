import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Button, Chip, Spinner } from "@heroui/react";
import { api } from "../lib/api";
import type { Imported, SettingsView, Skill, SkillDir } from "../lib/types";
import { Overlay } from "./Overlay";
import { ConfirmDialog } from "./ConfirmDialog";
import { FolderOpenIcon, SkillsIcon } from "./Icons";

const SKILL_SYSTEM_PROMPT = `You write SirVibe skills.

A skill is a Markdown file that tells an AI agent what good work looks like for
one kind of task. It carries judgement and standards, not code and not credentials.

Output ONLY the Markdown file. No preamble, no code fences around the whole thing.

Start with frontmatter:
---
name: kebab-case-name
description: One sentence on what this skill is for.
when_to_use: The situation that should trigger it.
---

Then a title and these sections, in this order:
## Purpose
## When to use
## Principles      (the judgement — what separates good from bad here)
## Workflow        (numbered, concrete steps)
## Constraints     (what must never happen)
## Quality criteria (how to tell it came out right)
## Failure conditions (the specific ways this work goes wrong)

Be specific and opinionated. Give real numbers, real thresholds, real command
shapes where they matter. Vague advice like "make it look good" is worthless —
say what to check, in what order, and what the target values are.`;

type Screen =
  | { name: "list" }
  | { name: "choose" }
  | { name: "editor"; title: string; skillName: string; content: string; note?: string }
  | { name: "ai" };

export function SkillsModal({
  settings,
  onSettings,
  onClose,
}: {
  settings: SettingsView;
  onSettings: (s: SettingsView) => void;
  onClose: () => void;
}) {
  const [skills, setSkills] = useState<Skill[] | null>(null);
  const [dirs, setDirs] = useState<SkillDir[]>([]);
  const [screen, setScreen] = useState<Screen>({ name: "list" });
  const [confirm, setConfirm] = useState<Skill | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  /** Names to mark as just-arrived, so a new skill is visible at a glance. */
  const [fresh, setFresh] = useState<string[]>([]);

  const refresh = async () => {
    setSkills(await api.listSkills());
    setDirs(await api.getSkillDirs());
  };
  useEffect(() => {
    void refresh();
  }, []);

  // Skills are files, and files change outside this window — dropped into the
  // folder, edited in another editor, added by the agent. Re-read whenever the
  // window comes back, so the list is never quietly out of date.
  useEffect(() => {
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  useEffect(() => {
    if (!fresh.length) return;
    const timer = setTimeout(() => setFresh([]), 6000);
    return () => clearTimeout(timer);
  }, [fresh]);

  const go = (next: Screen) => {
    setError(null);
    setScreen(next);
  };

  /** Land the skills, then say exactly what happened to each one. */
  const announce = (names: string[], message: string) => {
    setFresh(names);
    setNotice(message);
    setError(null);
  };

  const importSkill = async () => {
    const picked = await open({
      multiple: true,
      title: "Choose skill files",
      filters: [
        { name: "Markdown", extensions: ["md", "markdown"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
    const sources = typeof picked === "string" ? [picked] : Array.isArray(picked) ? picked : [];
    if (!sources.length) return;
    try {
      const report = await api.skillImport(sources);
      await refresh();
      go({ name: "list" });
      if (report.imported.length) {
        announce(
          report.imported.map((i) => i.name),
          describeImport(report.imported),
        );
      } else {
        setNotice(null);
      }
      if (report.failed.length) {
        setError(
          report.failed
            .map((f) => `${f.source.split("/").pop()}: ${f.reason}`)
            .join("\n"),
        );
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const edit = async (skill: Skill) => {
    try {
      const content = await api.skillRead(skill.path);
      go({
        name: "editor",
        title: `Edit ${skill.name}`,
        skillName: skill.name,
        content,
        note:
          skill.source === "bundled"
            ? "This is a built-in skill. Saving keeps the original intact and stores your version, which overrides it."
            : undefined,
      });
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async (skill: Skill) => {
    try {
      await api.skillDelete(skill.path);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setConfirm(null);
    }
  };

  if (screen.name === "editor") {
    return (
      <SkillEditor
        title={screen.title}
        initialName={screen.skillName}
        initialContent={screen.content}
        note={screen.note}
        onClose={onClose}
        onBack={() => go({ name: "list" })}
        onSaved={async (savedName) => {
          await refresh();
          go({ name: "list" });
          announce([savedName], `Saved “${savedName}”. The agent picks it up on your next message.`);
        }}
      />
    );
  }

  if (screen.name === "ai") {
    return (
      <AiSkillWriter
        ready={settings.api_key_set && Boolean(settings.model)}
        onClose={onClose}
        onBack={() => go({ name: "choose" })}
        onDrafted={(name, content) =>
          setScreen({
            name: "editor",
            title: "Review the draft",
            skillName: name,
            content,
            note: "Read it before saving. A skill is only as good as the judgement in it.",
          })
        }
      />
    );
  }

  if (screen.name === "choose") {
    return (
      <Overlay
        title="Add a skill"
        subtitle="A skill is a Markdown file telling the agent what good work looks like."
        onClose={onClose}
        footer={
          <Button variant="secondary" onPress={() => go({ name: "list" })}>
            Back
          </Button>
        }
      >
        <div className="flex flex-col gap-2.5">
          <Option
            title="Import a skill"
            body="Pick one or more Markdown files, or a skill folder. They are copied into your skills folder."
            onClick={importSkill}
          />
          <Option
            title="Write a skill"
            body="Open the editor and write it yourself."
            onClick={() =>
              go({
                name: "editor",
                title: "Write a skill",
                skillName: "",
                content: STARTER,
              })
            }
          />
          <Option
            title="Ask the AI to write it"
            body="Describe the standards you want and the model drafts the skill for you to review."
            onClick={() => go({ name: "ai" })}
          />
        </div>
        {error && (
          <p className="mt-3 whitespace-pre-line text-[13px] text-danger">{error}</p>
        )}
      </Overlay>
    );
  }

  return (
    <>
      <Overlay
        title={skills?.length ? `Skills (${skills.length})` : "Skills"}
        subtitle="Markdown files that tell the agent what good work looks like. Yours load exactly like the built-in ones."
        onClose={onClose}
        width="max-w-2xl"
        footer={
          <>
            <Button variant="ghost" onPress={() => void refresh()}>
              Refresh
            </Button>
            <Button
              variant="secondary"
              onPress={async () => api.revealPath(await api.ensureUserSkillsDir())}
            >
              <FolderOpenIcon />
              Open folder
            </Button>
            <Button variant="primary" onPress={() => go({ name: "choose" })}>
              Add Skill
            </Button>
          </>
        }
      >
        {error && (
          <p className="mb-3 whitespace-pre-line rounded-lg bg-danger/10 px-3 py-2 text-[13px] text-danger">
            {error}
          </p>
        )}
        {notice && (
          <p className="mb-3 rounded-lg bg-success/10 px-3 py-2 text-[13px] text-foreground">
            {notice}
          </p>
        )}
        {skills === null && (
          <div className="flex items-center gap-2 py-8 text-sm text-muted">
            <Spinner className="h-4 w-4" /> Loading…
          </div>
        )}
        {skills?.length === 0 && (
          <div className="py-10 text-center">
            <SkillsIcon className="mx-auto h-6 w-6 text-muted" />
            <p className="mt-2 text-sm font-medium">No skills yet</p>
            <p className="mx-auto mt-1 max-w-sm text-[13px] text-muted">
              Import a Markdown file, write one here, or drop files into your skills folder — the
              list updates when you come back to this window.
            </p>
          </div>
        )}

        <div className="flex flex-col gap-2">
          {skills?.map((s) => (
            <div
              key={s.path}
              className={`rounded-xl border px-3.5 py-3 transition-colors ${
                fresh.includes(s.name)
                  ? "border-success bg-success/[0.07]"
                  : "border-border"
              }`}
            >
              <div className="flex items-center gap-2">
                <span className="text-sm font-semibold text-foreground">{s.name}</span>
                <Chip
                  size="sm"
                  className={s.source === "bundled" ? "bg-accent/10 text-accent" : "bg-default"}
                >
                  {s.source}
                </Chip>
                {fresh.includes(s.name) && (
                  <Chip size="sm" className="bg-success/15 text-success">
                    just added
                  </Chip>
                )}
                <span className="flex-1" />
                <Button size="sm" variant="secondary" onPress={() => edit(s)}>
                  Edit
                </Button>
                {s.source !== "bundled" && (
                  <Button size="sm" variant="ghost" onPress={() => setConfirm(s)}>
                    Delete
                  </Button>
                )}
              </div>
              <p className="mt-1 text-[13px] leading-snug text-muted">{s.description}</p>
              {s.when_to_use && (
                <p className="mt-1 text-[12.5px] leading-snug text-muted/80">
                  <span className="font-medium">Use when:</span> {s.when_to_use}
                </p>
              )}
            </div>
          ))}
        </div>

        <details className="mt-3">
          <summary className="cursor-pointer text-xs text-muted">
            Folders searched ({dirs.length})
          </summary>
          <ul className="mt-2 flex flex-col gap-1">
            {dirs.map((d) => (
              <li
                key={d.path}
                className={`font-mono text-[11.5px] ${d.exists ? "text-muted" : "text-muted/50"}`}
              >
                {d.path} {!d.exists && "— not present"}
              </li>
            ))}
          </ul>
          <div className="mt-2 flex flex-wrap gap-1.5">
            <Button
              size="sm"
              variant="ghost"
              onPress={async () => {
                const picked = await open({ directory: true, title: "Add skills folder" });
                if (typeof picked === "string" && !settings.skill_dirs.includes(picked)) {
                  onSettings(
                    await api.updateSettings({ skill_dirs: [...settings.skill_dirs, picked] }),
                  );
                  await refresh();
                }
              }}
            >
              Add folder
            </Button>
            {settings.skill_dirs.map((d) => (
              <Button
                key={d}
                size="sm"
                variant="ghost"
                onPress={async () => {
                  onSettings(
                    await api.updateSettings({
                      skill_dirs: settings.skill_dirs.filter((x) => x !== d),
                    }),
                  );
                  await refresh();
                }}
              >
                Remove {d.split("/").pop()}
              </Button>
            ))}
          </div>
        </details>
      </Overlay>

      {confirm && (
        <ConfirmDialog
          title={`Delete “${confirm.name}”?`}
          body="The file is removed from your skills folder. This cannot be undone."
          onConfirm={() => remove(confirm)}
          onCancel={() => setConfirm(null)}
        />
      )}
    </>
  );
}

/** Say what landed, and say when something was replaced rather than added. */
function describeImport(imported: Imported[]): string {
  const names = imported.map((i) => `“${i.name}”`).join(", ");
  const replaced = imported.filter((i) => i.replaced).map((i) => `“${i.name}”`);
  const head =
    imported.length === 1 ? `Imported ${names}.` : `Imported ${imported.length} skills: ${names}.`;
  if (!replaced.length) return head;
  return `${head} ${replaced.join(", ")} replaced ${
    replaced.length === 1 ? "a skill" : "skills"
  } of the same name that ${replaced.length === 1 ? "was" : "were"} already there.`;
}

/** A skill is listed under its frontmatter name, not its file name. */
function nameInFrontmatter(content: string): string | null {
  return /^name:\s*(.+)$/m.exec(content)?.[1]?.trim().replace(/^["']|["']$/g, "") || null;
}

function Option({
  title,
  body,
  onClick,
}: {
  title: string;
  body: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="flex items-start gap-3 rounded-xl border border-border px-3.5 py-3 text-left transition-colors hover:border-accent hover:bg-default/50"
    >
      <SkillsIcon className="mt-0.5 h-4 w-4 shrink-0 text-muted" />
      <span>
        <span className="block text-sm font-medium text-foreground">{title}</span>
        <span className="mt-0.5 block text-[12.5px] leading-snug text-muted">{body}</span>
      </span>
    </button>
  );
}

function SkillEditor({
  title,
  initialName,
  initialContent,
  note,
  onClose,
  onBack,
  onSaved,
}: {
  title: string;
  initialName: string;
  initialContent: string;
  note?: string;
  onClose: () => void;
  onBack: () => void;
  onSaved: (name: string) => void;
}) {
  const [name, setName] = useState(initialName);
  const [content, setContent] = useState(initialContent);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.skillWrite(name, content);
      onSaved(nameInFrontmatter(content) ?? name.trim());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Overlay
      title={title}
      subtitle="Saved to your skills folder as Markdown. The agent picks it up on the next message."
      onClose={onClose}
      width="max-w-3xl"
      footer={
        <>
          <Button variant="secondary" onPress={onBack} isDisabled={busy}>
            Cancel
          </Button>
          <Button variant="primary" onPress={save} isDisabled={busy || !name.trim()}>
            {busy ? "Saving…" : "Save skill"}
          </Button>
        </>
      }
    >
      {note && (
        <p className="mb-3 rounded-lg bg-accent/[0.08] px-3 py-2 text-[12.5px] text-foreground">
          {note}
        </p>
      )}
      <label className="block">
        <span className="mb-1.5 block text-[13px] font-medium">Skill name</span>
        <input
          className="w-full rounded-lg border border-field-border bg-field px-3 py-1.5 text-sm outline-none focus:border-accent"
          placeholder="my-house-style"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
      </label>
      <label className="mt-3 block">
        <span className="mb-1.5 block text-[13px] font-medium">Markdown</span>
        <textarea
          spellCheck={false}
          value={content}
          onChange={(e) => setContent(e.target.value)}
          className="h-[46vh] w-full resize-none rounded-lg border border-field-border bg-field px-3 py-2.5 font-mono text-[12.5px] leading-relaxed outline-none focus:border-accent"
        />
      </label>
      {error && <p className="mt-2 text-[13px] text-danger">{error}</p>}
    </Overlay>
  );
}

function AiSkillWriter({
  ready,
  onClose,
  onBack,
  onDrafted,
}: {
  ready: boolean;
  onClose: () => void;
  onBack: () => void;
  onDrafted: (name: string, content: string) => void;
}) {
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const generate = async () => {
    setBusy(true);
    setError(null);
    try {
      const draft = await api.generateText(prompt, SKILL_SYSTEM_PROMPT);
      const name = /^name:\s*(.+)$/m.exec(draft)?.[1]?.trim() ?? "new-skill";
      onDrafted(name, draft.trim());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Overlay
      title="Ask the AI to write a skill"
      subtitle="Describe the work and the standards it should hold. You review the draft before it is saved."
      onClose={onClose}
      footer={
        <>
          <Button variant="secondary" onPress={onBack} isDisabled={busy}>
            Back
          </Button>
          <Button
            variant="primary"
            onPress={generate}
            isDisabled={busy || !prompt.trim() || !ready}
          >
            {busy ? "Writing…" : "Write it"}
          </Button>
        </>
      }
    >
      {!ready && (
        <p className="mb-3 rounded-lg bg-warning/10 px-3 py-2 text-[13px] text-warning-foreground">
          Set your API key and model in Settings first — this uses the same model as the agent.
        </p>
      )}
      <textarea
        autoFocus
        placeholder="e.g. How our channel cuts podcast clips: hook in the first two seconds, never cut mid-word, 30–60 seconds, vertical, captions burned in."
        value={prompt}
        onChange={(e) => setPrompt(e.target.value)}
        className="h-40 w-full resize-none rounded-lg border border-field-border bg-field px-3 py-2.5 text-sm outline-none focus:border-accent"
      />
      <p className="mt-2 text-[12px] text-muted">
        The more specific you are about thresholds and what counts as failure, the more useful the
        skill will be.
      </p>
      {error && <p className="mt-2 text-[13px] text-danger">{error}</p>}
    </Overlay>
  );
}

const STARTER = `---
name: my-skill
description: One sentence on what this skill is for.
when_to_use: The situation that should trigger it.
---

# My Skill

## Purpose

## When to use

## Principles

## Workflow

1.

## Constraints

## Quality criteria

## Failure conditions
`;
