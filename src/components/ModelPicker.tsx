import { useEffect, useMemo, useState } from "react";
import { Button, Chip, Spinner } from "@heroui/react";
import { api } from "../lib/api";
import type { ModelInfo } from "../lib/types";
import { Overlay } from "./Overlay";
import { CheckIcon, SearchIcon } from "./Icons";

type Kind = "all" | "text" | "image" | "audio" | "video";

const KINDS: { id: Kind; label: string }[] = [
  { id: "all", label: "All" },
  { id: "text", label: "Text" },
  { id: "image", label: "Image" },
  { id: "audio", label: "Audio" },
  { id: "video", label: "Video" },
];

/** OpenRouter namespaces models by organisation; the prefix is the provider. */
const providerLabel = (id: string) =>
  id
    .split("-")
    .map((w) => (w.length <= 2 ? w.toUpperCase() : w[0].toUpperCase() + w.slice(1)))
    .join(" ");

const shortName = (m: ModelInfo) => {
  const after = m.name.includes(":") ? m.name.split(":").slice(1).join(":") : m.name;
  return after.trim() || m.id;
};

const perMillion = (price: string) => {
  const value = Number(price);
  if (!price || Number.isNaN(value)) return "";
  if (value === 0) return "free";
  const dollars = value * 1_000_000;
  return dollars < 1 ? `$${dollars.toFixed(2)}/M` : `$${dollars.toFixed(dollars < 10 ? 1 : 0)}/M`;
};

export function ModelPicker({
  current,
  onPick,
  onClose,
}: {
  current: string;
  onPick: (id: string) => void;
  onClose: () => void;
}) {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<Kind>("all");
  const [toolsOnly, setToolsOnly] = useState(true);
  const [openProvider, setOpenProvider] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    api
      .listModels()
      .then(setModels)
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, []);

  // The agent needs tool calling; a model that makes pictures never will, so
  // the filter has to step aside as soon as you are not shopping for text.
  const toolsFilterApplies = kind === "all" || kind === "text";

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const matched = models.filter((m) => {
      if (kind !== "all" && !m.output_modalities.includes(kind)) return false;
      if (toolsOnly && toolsFilterApplies && !m.supports_tools) return false;
      if (!q) return true;
      return (
        m.id.toLowerCase().includes(q) ||
        m.name.toLowerCase().includes(q) ||
        m.description.toLowerCase().includes(q)
      );
    });

    const byProvider = new Map<string, ModelInfo[]>();
    for (const m of matched) {
      const bucket = byProvider.get(m.provider) ?? [];
      bucket.push(m);
      byProvider.set(m.provider, bucket);
    }
    return [...byProvider.entries()]
      .map(([provider, items]) => ({
        provider,
        items: items.sort((a, b) => shortName(a).localeCompare(shortName(b))),
      }))
      .sort((a, b) => b.items.length - a.items.length || a.provider.localeCompare(b.provider));
  }, [models, query, kind, toolsOnly, toolsFilterApplies]);

  const total = groups.reduce((n, g) => n + g.items.length, 0);
  // Searching means you want to see hits, not go hunting through folded rows.
  const searching = Boolean(query.trim());
  const expanded = (provider: string) =>
    searching || openProvider === provider || groups.length === 1;

  return (
    <Overlay
      title="Model"
      subtitle="Every model your OpenRouter key can reach, grouped by who makes it. The agent itself needs a text model with tool calling; the others are for run_model."
      onClose={onClose}
      width="max-w-2xl"
    >
      <div className="mb-3 flex items-center gap-2 rounded-xl border border-field-border bg-field px-3">
        <SearchIcon className="h-4 w-4 text-muted" />
        <input
          autoFocus
          placeholder="Search, or type any OpenRouter model id"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && query.includes("/")) onPick(query.trim());
            if (e.key === "Escape") onClose();
          }}
          className="flex-1 bg-transparent py-2 text-sm text-foreground outline-none placeholder:text-field-placeholder"
        />
      </div>

      <div className="mb-3 flex flex-wrap items-center gap-1.5">
        {KINDS.map((k) => (
          <button
            key={k.id}
            onClick={() => setKind(k.id)}
            className={`rounded-full px-3 py-1 text-[12.5px] transition-colors ${
              kind === k.id
                ? "bg-accent text-accent-foreground"
                : "border border-border text-muted hover:bg-default/60"
            }`}
          >
            {k.label}
          </button>
        ))}
        <span className="flex-1" />
        {toolsFilterApplies && (
          <label className="flex cursor-pointer items-center gap-1.5 text-[12.5px] text-muted">
            <input
              type="checkbox"
              checked={toolsOnly}
              onChange={(e) => setToolsOnly(e.target.checked)}
              className="accent-accent"
            />
            Tool calling only
          </label>
        )}
      </div>

      {loading && (
        <div className="flex items-center gap-2 py-8 text-sm text-muted">
          <Spinner className="h-4 w-4" /> Loading models from OpenRouter…
        </div>
      )}
      {error && <p className="py-4 text-sm text-danger">{error}</p>}

      {!loading && !error && (
        <p className="mb-2 text-[12px] text-muted">
          {total} {total === 1 ? "model" : "models"} from {groups.length}{" "}
          {groups.length === 1 ? "provider" : "providers"}
        </p>
      )}

      <div className="flex flex-col gap-1.5">
        {groups.map(({ provider, items }) => (
          <div key={provider} className="overflow-hidden rounded-xl border border-border">
            <button
              onClick={() => setOpenProvider(expanded(provider) ? null : provider)}
              className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-default/50"
            >
              <span className="text-[13px] font-semibold text-foreground">
                {providerLabel(provider)}
              </span>
              <span className="text-[11.5px] text-muted">{items.length}</span>
              <span className="flex-1" />
              {items.some((m) => m.id === current) && (
                <Chip size="sm" className="bg-accent/10 text-accent">
                  in use
                </Chip>
              )}
            </button>

            {expanded(provider) && (
              <div className="flex flex-col border-t border-border px-1.5 py-1.5">
                {items.map((m) => (
                  <button
                    key={m.id}
                    onClick={() => onPick(m.id)}
                    className={`flex items-center gap-3 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-default/70 ${
                      m.id === current ? "bg-accent/[0.07]" : ""
                    }`}
                  >
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[13px] text-foreground">
                        {shortName(m)}
                      </span>
                      <span className="block truncate font-mono text-[11px] text-muted">
                        {m.id}
                      </span>
                    </span>
                    <span className="shrink-0 text-right text-[11px] tabular-nums text-muted">
                      <span className="block">
                        {m.output_modalities.filter((o) => o !== "text").join(", ") ||
                          (m.context_length
                            ? `${Math.round(m.context_length / 1000)}k context`
                            : "")}
                      </span>
                      <span className="block">{perMillion(m.prompt_price)}</span>
                    </span>
                    {m.id === current && <CheckIcon className="h-4 w-4 shrink-0 text-accent" />}
                  </button>
                ))}
              </div>
            )}
          </div>
        ))}

        {!loading && !total && (
          <p className="py-6 text-center text-sm text-muted">
            {query.trim()
              ? `No match. Press Enter to use “${query.trim()}” as a model id anyway.`
              : "No models match these filters."}
          </p>
        )}
      </div>

      <div className="pt-3">
        <Button variant="secondary" size="sm" onPress={onClose}>
          Close
        </Button>
      </div>
    </Overlay>
  );
}
