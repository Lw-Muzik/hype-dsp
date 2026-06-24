import { useCallback, useEffect, useRef, useState } from "react";
import { BookmarkPlus, Download, Play, Trash2, Upload } from "lucide-react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import { toast } from "@/stores/toast";
import {
  chainPresetDelete,
  chainPresetExport,
  chainPresetImport,
  chainPresetList,
  chainPresetSave,
  ipcErrorMessage,
} from "@/lib/ipc";
import type { ChainPreset } from "@/lib/types";

/** Whole-chain DSP preset manager: save, apply, delete, import, and export. */
export function PresetsCard() {
  const applyChainPreset = useEngineStore((s) => s.applyChainPreset);

  const [presets, setPresets] = useState<ChainPreset[]>([]);
  const [newName, setNewName] = useState("");
  const [saving, setSaving] = useState(false);
  const [importing, setImporting] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const refresh = useCallback(async () => {
    try {
      setPresets(await chainPresetList());
    } catch (e) {
      toast.error(`Couldn't load presets: ${ipcErrorMessage(e)}`);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleSave = async () => {
    const name = newName.trim();
    if (!name) return;
    setSaving(true);
    try {
      await chainPresetSave(name);
      await refresh();
      setNewName("");
      inputRef.current?.focus();
      toast.success(`Preset "${name}" saved.`);
    } catch (e) {
      toast.error(`Save failed: ${ipcErrorMessage(e)}`);
    } finally {
      setSaving(false);
    }
  };

  const handleImport = async () => {
    let path: string | string[] | null;
    try {
      path = await open({
        multiple: false,
        filters: [{ name: "Chain Preset", extensions: ["json"] }],
      });
    } catch {
      return; // dialog cancelled
    }
    if (typeof path !== "string") return;
    setImporting(true);
    try {
      const preset = await chainPresetImport(path);
      await refresh();
      toast.success(`Imported "${preset.name}".`);
    } catch (e) {
      toast.error(`Import failed: ${ipcErrorMessage(e)}`);
    } finally {
      setImporting(false);
    }
  };

  const handleApply = async (preset: ChainPreset) => {
    try {
      await applyChainPreset(preset.id);
      toast.success(`Applied "${preset.name}".`);
    } catch (e) {
      toast.error(`Apply failed: ${ipcErrorMessage(e)}`);
    }
  };

  const handleExport = async (preset: ChainPreset) => {
    let path: string | null;
    try {
      path = await save({
        defaultPath: `${preset.name}.json`,
        filters: [{ name: "Chain Preset", extensions: ["json"] }],
      });
    } catch {
      return; // dialog cancelled
    }
    if (!path) return;
    try {
      await chainPresetExport(preset.id, path);
      toast.success(`Exported "${preset.name}".`);
    } catch (e) {
      toast.error(`Export failed: ${ipcErrorMessage(e)}`);
    }
  };

  const handleDelete = async (preset: ChainPreset) => {
    try {
      await chainPresetDelete(preset.id);
      await refresh();
    } catch (e) {
      toast.error(`Delete failed: ${ipcErrorMessage(e)}`);
    }
  };

  return (
    <Card title="Chain presets" icon={BookmarkPlus}>
      <div className="flex flex-col gap-4">
        {/* Save current state as a named preset */}
        <div className="flex items-center gap-2">
          <input
            ref={inputRef}
            type="text"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void handleSave();
            }}
            placeholder="Preset name…"
            aria-label="New preset name"
            className="min-w-0 flex-1 rounded-control border border-border bg-surface px-3 py-2 text-sm placeholder:text-text-faint focus:border-accent/60 focus:outline-none"
          />
          <Button
            variant="primary"
            onClick={() => void handleSave()}
            disabled={saving || !newName.trim()}
          >
            {saving ? "Saving…" : "Save"}
          </Button>
        </div>

        {/* Import button */}
        <div>
          <Button
            variant="secondary"
            onClick={() => void handleImport()}
            disabled={importing}
          >
            <Upload className="size-4" aria-hidden="true" />
            {importing ? "Importing…" : "Import…"}
          </Button>
        </div>

        {/* Preset list */}
        {presets.length === 0 ? (
          <p className="text-sm text-text-faint">No presets saved yet.</p>
        ) : (
          <ul className="flex flex-col gap-1" role="list">
            {presets.map((preset) => (
              <li
                key={preset.id}
                className="flex items-center gap-2 rounded-control border border-border bg-surface px-3 py-2"
              >
                <span className="min-w-0 flex-1 truncate text-sm">{preset.name}</span>
                <button
                  type="button"
                  title={`Apply "${preset.name}"`}
                  aria-label={`Apply preset ${preset.name}`}
                  onClick={() => void handleApply(preset)}
                  className="rounded p-1 text-text-muted transition-colors hover:text-accent"
                >
                  <Play className="size-3.5" aria-hidden="true" />
                </button>
                <button
                  type="button"
                  title={`Export "${preset.name}"`}
                  aria-label={`Export preset ${preset.name}`}
                  onClick={() => void handleExport(preset)}
                  className="rounded p-1 text-text-muted transition-colors hover:text-text"
                >
                  <Download className="size-3.5" aria-hidden="true" />
                </button>
                <button
                  type="button"
                  title={`Delete "${preset.name}"`}
                  aria-label={`Delete preset ${preset.name}`}
                  onClick={() => void handleDelete(preset)}
                  className="rounded p-1 text-text-muted transition-colors hover:text-danger"
                >
                  <Trash2 className="size-3.5" aria-hidden="true" />
                </button>
              </li>
            ))}
          </ul>
        )}

        {/* IR path caveat — imported presets from another machine may have a
            missing IR path; applying still works, convolver just has no IR. */}
        <p className="text-xs text-text-faint">
          Applying preserves your current volume. IR file paths in exported
          presets are machine-specific and may not resolve on import.
        </p>
      </div>
    </Card>
  );
}
