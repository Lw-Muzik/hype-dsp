import { useState } from "react";
import { Check, Plus, X } from "lucide-react";
import { cn } from "@/lib/cn";
import type { EqPreset } from "@/lib/types";

interface PresetBarProps {
  presets: EqPreset[];
  activeId: string | null;
  onApply: (id: string) => void;
  onSave: (name: string) => void;
  onDelete: (id: string) => void;
}

/** Horizontal preset selector with inline save and per-custom delete. */
export function PresetBar({
  presets,
  activeId,
  onApply,
  onSave,
  onDelete,
}: PresetBarProps) {
  const [saving, setSaving] = useState(false);
  const [name, setName] = useState("");

  const commitSave = () => {
    const trimmed = name.trim();
    if (trimmed) onSave(trimmed);
    setName("");
    setSaving(false);
  };

  return (
    <div className="flex items-center gap-2 overflow-x-auto pb-1">
      {presets.map((preset) => {
        const active = preset.id === activeId;
        return (
          <div
            key={preset.id}
            className={cn(
              "group flex shrink-0 items-center rounded-full border text-sm transition-colors",
              active
                ? "border-accent/40 bg-accent-muted text-accent-strong"
                : "border-border bg-surface-raised text-text-muted hover:text-text",
            )}
          >
            <button
              type="button"
              onClick={() => onApply(preset.id)}
              className="py-1.5 pl-3 pr-2"
            >
              {preset.name}
            </button>
            {!preset.builtin && (
              <button
                type="button"
                aria-label={`Delete preset ${preset.name}`}
                onClick={() => onDelete(preset.id)}
                className="pr-2.5 text-text-faint hover:text-danger"
              >
                <X className="size-3.5" aria-hidden="true" />
              </button>
            )}
            {preset.builtin && <span className="pr-3" />}
          </div>
        );
      })}

      {saving ? (
        <div className="flex shrink-0 items-center gap-1 rounded-full border border-accent/40 bg-surface-raised pl-3 pr-1">
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitSave();
              if (e.key === "Escape") setSaving(false);
            }}
            placeholder="Preset name"
            className="w-32 bg-transparent py-1.5 text-sm outline-none placeholder:text-text-faint"
          />
          <button
            type="button"
            aria-label="Confirm save"
            onClick={commitSave}
            className="rounded-full p-1 text-success hover:bg-surface-overlay"
          >
            <Check className="size-4" aria-hidden="true" />
          </button>
        </div>
      ) : (
        <button
          type="button"
          onClick={() => setSaving(true)}
          className="flex shrink-0 items-center gap-1 rounded-full border border-dashed border-border px-3 py-1.5 text-sm text-text-muted hover:text-text"
        >
          <Plus className="size-3.5" aria-hidden="true" />
          Save
        </button>
      )}
    </div>
  );
}
