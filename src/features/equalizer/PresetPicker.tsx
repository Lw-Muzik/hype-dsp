import { useMemo, useState } from "react";
import { Check, Loader2, Plus, Trash2, X } from "lucide-react";
import { Combobox } from "@/components/Combobox";
import type { ComboItem } from "@/components/Combobox";
import type { EqPreset } from "@/lib/types";

interface PresetPickerProps {
  /** Built-in + saved-custom graphic-EQ presets. */
  presets: EqPreset[];
  activeId: string | null;
  onApply: (id: string) => void;
  onSave: (name: string) => void;
  onDelete: (id: string) => void;

  /** Bundled ViPER DDC curve names (600+). */
  ddcNames: string[];
  ddcLoading: boolean;
  /** The DDC curve applied most recently (shown in the dropdown), or null. */
  appliedDdc: string | null;
  applyingDdc: boolean;
  onApplyDdc: (name: string) => void;
}

/**
 * One place to choose a sound profile: a searchable dropdown for the built-in /
 * custom graphic-EQ presets (with save + delete for your own), and a second for
 * the bundled ViPER DDC curves. Replaces the old scattered pill bar + hidden
 * "DDC presets…" panel so every selector lives together.
 */
export function PresetPicker({
  presets,
  activeId,
  onApply,
  onSave,
  onDelete,
  ddcNames,
  ddcLoading,
  appliedDdc,
  applyingDdc,
  onApplyDdc,
}: PresetPickerProps) {
  const [saving, setSaving] = useState(false);
  const [name, setName] = useState("");

  const presetItems: ComboItem[] = useMemo(
    () =>
      presets.map((p) => ({
        id: p.id,
        label: p.name,
        sublabel: p.builtin ? "Built-in" : "Custom",
      })),
    [presets],
  );

  const ddcItems: ComboItem[] = useMemo(
    () => ddcNames.map((n) => ({ id: n, label: n })),
    [ddcNames],
  );

  const active = presets.find((p) => p.id === activeId) ?? null;
  const canDelete = active != null && !active.builtin;

  const commitSave = () => {
    const trimmed = name.trim();
    if (trimmed) onSave(trimmed);
    setName("");
    setSaving(false);
  };

  return (
    <div className="grid gap-x-5 gap-y-4 rounded-card border border-border bg-surface/40 p-3 sm:grid-cols-2">
      {/* ---- Graphic-EQ preset ---- */}
      <div className="flex min-w-0 flex-col gap-1.5">
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs font-medium uppercase tracking-wide text-text-faint">
            Equalizer preset
          </span>
          <div className="flex items-center gap-1">
            {canDelete && !saving && (
              <button
                type="button"
                onClick={() => onDelete(active.id)}
                aria-label={`Delete preset ${active.name}`}
                className="flex items-center gap-1 rounded-control px-1.5 py-0.5 text-xs text-text-faint transition-colors hover:text-danger"
              >
                <Trash2 className="size-3.5" aria-hidden="true" />
                Delete
              </button>
            )}
            {!saving && (
              <button
                type="button"
                onClick={() => setSaving(true)}
                className="flex items-center gap-1 rounded-control px-1.5 py-0.5 text-xs text-text-muted transition-colors hover:text-text"
              >
                <Plus className="size-3.5" aria-hidden="true" />
                Save current
              </button>
            )}
          </div>
        </div>

        {saving ? (
          <div className="flex items-center gap-1 rounded-control border border-accent bg-surface px-2 focus-within:border-accent">
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitSave();
                if (e.key === "Escape") {
                  setName("");
                  setSaving(false);
                }
              }}
              placeholder="Name this preset"
              className="w-full bg-transparent py-2.5 text-sm placeholder:text-text-faint"
            />
            <button
              type="button"
              aria-label="Save preset"
              onClick={commitSave}
              disabled={name.trim() === ""}
              className="rounded-control p-1 text-success transition-colors hover:bg-surface-overlay disabled:opacity-40"
            >
              <Check className="size-4" aria-hidden="true" />
            </button>
            <button
              type="button"
              aria-label="Cancel"
              onClick={() => {
                setName("");
                setSaving(false);
              }}
              className="rounded-control p-1 text-text-faint transition-colors hover:text-text"
            >
              <X className="size-4" aria-hidden="true" />
            </button>
          </div>
        ) : (
          <Combobox
            items={presetItems}
            value={activeId}
            onSelect={onApply}
            placeholder="Choose a preset"
            searchPlaceholder="Search presets…"
            emptyText="No presets"
          />
        )}
      </div>

      {/* ---- ViPER DDC curve ---- */}
      <div className="flex min-w-0 flex-col gap-1.5">
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs font-medium uppercase tracking-wide text-text-faint">
            ViPER DDC curve
          </span>
          {applyingDdc && (
            <span className="flex items-center gap-1 text-xs text-text-faint">
              <Loader2 className="size-3 animate-spin" aria-hidden="true" />
              Applying…
            </span>
          )}
        </div>
        <Combobox
          items={ddcItems}
          value={appliedDdc}
          onSelect={onApplyDdc}
          placeholder={ddcLoading ? "Loading curves…" : "Choose a DDC curve"}
          searchPlaceholder={
            ddcNames.length ? `Search ${ddcNames.length} curves…` : "Search curves…"
          }
          emptyText={ddcLoading ? "Loading…" : "No curves match"}
        />
      </div>
    </div>
  );
}
