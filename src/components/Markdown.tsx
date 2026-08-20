import { useState, type JSX } from "react";
import { CopyIcon } from "./Icons";

/**
 * Just enough Markdown for agent replies: fenced code, inline code, bold,
 * headings and list items. Not a general renderer, and deliberately not a
 * dependency.
 */
export function Markdown({ text }: { text: string }) {
  const blocks = text.split(/```/);
  return (
    <div className="text-[15px] leading-[1.65] text-foreground">
      {blocks.map((block, i) =>
        i % 2 === 1 ? <CodeBlock key={i} raw={block} /> : <Prose key={i} text={block} />,
      )}
    </div>
  );
}

function CodeBlock({ raw }: { raw: string }) {
  const [copied, setCopied] = useState(false);
  const match = /^([a-zA-Z0-9+#-]*)\n([\s\S]*)$/.exec(raw);
  const lang = (match?.[1] ?? "").toUpperCase();
  const code = match ? match[2] : raw;

  const copy = async () => {
    await navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="my-3 overflow-hidden rounded-xl bg-background-secondary">
      <div className="flex items-center justify-between px-3 py-1.5">
        <span className="font-mono text-[10.5px] tracking-wider text-muted">{lang || "TEXT"}</span>
        <button
          onClick={copy}
          title="Copy code"
          className="rounded-md p-1 text-muted transition-colors hover:text-foreground"
        >
          {copied ? <span className="text-[11px] text-success">Copied</span> : <CopyIcon className="h-3.5 w-3.5" />}
        </button>
      </div>
      <pre className="overflow-x-auto px-3 pb-3 font-mono text-[12.5px] leading-relaxed">
        <code>{code.replace(/\n$/, "")}</code>
      </pre>
    </div>
  );
}

function Prose({ text }: { text: string }) {
  const lines = text.split("\n");
  const out: JSX.Element[] = [];
  let paragraph: string[] = [];

  const flush = () => {
    if (!paragraph.length) return;
    out.push(
      <p key={`p${out.length}`} className="mb-3 whitespace-pre-wrap last:mb-0">
        {inline(paragraph.join("\n"))}
      </p>,
    );
    paragraph = [];
  };

  for (const line of lines) {
    const heading = /^(#{1,4})\s+(.*)$/.exec(line);
    const bullet = /^\s*[-*]\s+(.*)$/.exec(line);
    const numbered = /^\s*(\d+)\.\s+(.*)$/.exec(line);
    if (!line.trim()) {
      flush();
    } else if (heading) {
      flush();
      out.push(
        <div key={`h${out.length}`} className="mt-4 mb-1.5 font-semibold first:mt-0">
          {inline(heading[2])}
        </div>,
      );
    } else if (bullet || numbered) {
      flush();
      const marker = bullet ? "•" : `${numbered![1]}.`;
      const content = bullet ? bullet[1] : numbered![2];
      out.push(
        <div key={`l${out.length}`} className="mb-1 flex gap-2.5">
          <span className="shrink-0 text-muted">{marker}</span>
          <span>{inline(content)}</span>
        </div>,
      );
    } else {
      paragraph.push(line);
    }
  }
  flush();
  return <>{out}</>;
}

function inline(text: string): JSX.Element[] {
  const parts: JSX.Element[] = [];
  const pattern = /(`[^`]+`|\*\*[^*]+\*\*)/g;
  let last = 0;
  let match: RegExpExecArray | null;
  let key = 0;
  while ((match = pattern.exec(text)) !== null) {
    if (match.index > last) parts.push(<span key={key++}>{text.slice(last, match.index)}</span>);
    const token = match[0];
    if (token.startsWith("`")) {
      parts.push(
        <code
          key={key++}
          className="rounded-md bg-background-secondary px-1.5 py-0.5 font-mono text-[0.86em]"
        >
          {token.slice(1, -1)}
        </code>,
      );
    } else {
      parts.push(
        <strong key={key++} className="font-semibold">
          {token.slice(2, -2)}
        </strong>,
      );
    }
    last = match.index + token.length;
  }
  if (last < text.length) parts.push(<span key={key++}>{text.slice(last)}</span>);
  return parts;
}
