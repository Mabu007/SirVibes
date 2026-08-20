import { useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Button, Spinner } from "@heroui/react";
import { api } from "../lib/api";
import type { Artifact } from "../lib/types";
import { CopyIcon, FileIcon, FolderOpenIcon } from "./Icons";

/**
 * On Linux the webview reaches `asset://` through its network layer but plays
 * media through GStreamer, which cannot see custom schemes — so a <video src>
 * pointed straight at the asset URL fails with SRC_NOT_SUPPORTED even though
 * the file is served correctly. Fetching once into a blob URL sidesteps that,
 * at the cost of holding the file in memory, so only do it below this size.
 */
const MAX_INLINE_BYTES = 150 * 1024 * 1024;

const size = (n: number) => {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
};

export function ArtifactStrip({ items }: { items: Artifact[] }) {
  return (
    <div className="mt-3 flex flex-col gap-3">
      <div className="text-xs font-medium text-muted">
        {items.length} {items.length === 1 ? "artifact" : "artifacts"}
      </div>
      {items.map((a) => (
        <ArtifactCard key={a.absolute_path} artifact={a} />
      ))}
    </div>
  );
}

type Preview = { state: "idle" | "loading" | "ready" | "failed" | "too-large"; url?: string };

function ArtifactCard({ artifact }: { artifact: Artifact }) {
  const card = useRef<HTMLDivElement>(null);
  const [copied, setCopied] = useState(false);
  const [preview, setPreview] = useState<Preview>({ state: "idle" });
  const src = useMemo(() => convertFileSrc(artifact.absolute_path), [artifact.absolute_path]);

  const isMedia = artifact.kind === "video" || artifact.kind === "audio";
  const isImage = artifact.kind === "image";

  useEffect(() => {
    if (!isMedia) return;
    if (artifact.size > MAX_INLINE_BYTES) {
      setPreview({ state: "too-large" });
      return;
    }
    let cancelled = false;
    let url: string | undefined;
    setPreview({ state: "loading" });
    (async () => {
      try {
        const response = await fetch(src);
        if (!response.ok) throw new Error(`status ${response.status}`);
        const blob = await response.blob();
        if (cancelled) return;
        url = URL.createObjectURL(blob);
        setPreview({ state: "ready", url });
      } catch {
        if (!cancelled) setPreview({ state: "failed" });
      }
    })();
    return () => {
      cancelled = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [src, isMedia, artifact.size]);

  const copy = async () => {
    await navigator.clipboard.writeText(artifact.absolute_path);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  // A video only reports its height once metadata arrives, after the scroller
  // has already settled — so bring the finished card back into view.
  const revealCard = () => card.current?.scrollIntoView({ block: "end", behavior: "smooth" });

  return (
    <div ref={card} className="overflow-hidden rounded-xl border border-border bg-background">
      {isMedia && preview.state === "loading" && (
        <div className="flex items-center justify-center gap-2 border-b border-border py-10 text-sm text-muted">
          <Spinner className="h-4 w-4" /> Loading preview…
        </div>
      )}

      {isMedia && preview.state === "ready" && preview.url && (
        <div className="border-b border-border">
          {artifact.kind === "video" ? (
            <video
              src={preview.url}
              controls
              preload="metadata"
              onLoadedMetadata={revealCard}
              onError={() => setPreview({ state: "failed" })}
              className="max-h-[340px] w-full bg-black"
            />
          ) : (
            <audio
              src={preview.url}
              controls
              onLoadedMetadata={revealCard}
              className="w-full px-3 py-3"
            />
          )}
        </div>
      )}

      {isImage && (
        <div className="border-b border-border bg-black/[0.03]">
          <img
            src={src}
            alt={artifact.name}
            onLoad={revealCard}
            onError={() => setPreview({ state: "failed" })}
            className="max-h-[340px] w-full object-contain"
          />
        </div>
      )}

      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 px-3 py-2.5">
        <FileIcon className="h-4 w-4 shrink-0 text-muted" />
        <span className="min-w-0 flex-1 truncate font-mono text-[13px] text-foreground">
          {artifact.path}
        </span>
        <span className="shrink-0 text-xs tabular-nums text-muted">{size(artifact.size)}</span>
        <div className="flex shrink-0 items-center gap-1.5">
          <Button size="sm" variant="outline" onPress={() => api.openPath(artifact.absolute_path)}>
            Open
          </Button>
          <Button
            size="sm"
            variant="ghost"
            isIconOnly
            aria-label="Reveal in folder"
            onPress={() => api.revealPath(artifact.absolute_path)}
          >
            <FolderOpenIcon />
          </Button>
          <Button size="sm" variant="ghost" isIconOnly aria-label="Copy path" onPress={copy}>
            {copied ? <span className="text-xs text-success">✓</span> : <CopyIcon />}
          </Button>
        </div>
      </div>

      {preview.state === "too-large" && (
        <div className="border-t border-border px-3 py-2 text-xs text-muted">
          Too large to play inline ({size(artifact.size)}) — Open plays it in your usual player.
        </div>
      )}
      {preview.state === "failed" && (
        <div className="border-t border-border px-3 py-2 text-xs text-muted">
          Can't preview this one here — Open plays it in your usual player.
        </div>
      )}
    </div>
  );
}
