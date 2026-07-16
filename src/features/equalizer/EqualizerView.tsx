import { memo, useCallback, useEffect, useState } from "react";
import { RotateCcw } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { Switch } from "@/components/Switch";
import { Slider } from "@/components/Slider";
import { EqBandSlider } from "@/components/EqBandSlider";
import { EqVisualizer } from "@/features/equalizer/EqVisualizer";
import { PresetPicker } from "@/features/equalizer/PresetPicker";
import { useEngineStore } from "@/stores/engine";
<<<<<<< HEAD
import { eqApplyPreset, eqDelete, eqListPresets, eqSaveCustom, ipcErrorMessage, ddcList } from "@/lib/ipc";
=======
import { autoeqSearch, eqApplyPreset, eqDelete, eqListPresets, eqSaveCustom, ipcErrorMessage } from "@/lib/ipc";
>>>>>>> feat/autoeq-fetch
import { BAND_COUNT, ISO_CENTERS_HZ } from "@/lib/types";
import type { AutoEqEntry, EqPreset } from "@/lib/types";
import { formatDb, formatHz } from "@/lib/format";
import { toast } from "@/stores/toast";

const DB_MIN = -12;
const DB_MAX = 12;

interface BandColumnProps {
  index: number;
  value: number;
  /** Stable store action — (bandIndex, valueDb). */
  onBandChange: (index: number, valueDb: number) => void;
}

/**
 * One fader column, memoized with a stable per-band handler so a drag on one
 * band (or a live-spectrum frame elsewhere) doesn't re-reconcile all 31 rows.
 */
const BandColumn = memo(function BandColumn({
  index,
  value,
  onBandChange,
}: BandColumnProps) {
  const label = formatHz(ISO_CENTERS_HZ[index] ?? 20);
  const onChange = useCallback(
    (v: number) => onBandChange(index, v),
    [onBandChange, index],
  );
  return (
    <div className="flex flex-1 flex-col items-center gap-1">
      <EqBandSlider
        value={value}
        min={DB_MIN}
        max={DB_MAX}
        label={label}
        onChange={onChange}
      />
      <span className="h-3 text-[8px] leading-none text-text-faint">
        {index % 3 === 0 ? label : ""}
      </span>
    </div>
  );
});

export function EqualizerView() {
  const route = routeById("equalizer");

  const bands = useEngineStore((s) => s.state.eq.bands);
  const preGain = useEngineStore((s) => s.state.eq.preGain);
  const enabled = useEngineStore((s) => s.state.eq.enabled);
  const activePresetId = useEngineStore((s) => s.state.activePresetId);
  const setBand = useEngineStore((s) => s.setBand);
  const setBands = useEngineStore((s) => s.setBands);
  const setPreGain = useEngineStore((s) => s.setPreGain);
  const setEqEnabled = useEngineStore((s) => s.setEqEnabled);
  const applyPreset = useEngineStore((s) => s.applyPreset);
  const importGraphicEq = useEngineStore((s) => s.importGraphicEq);
<<<<<<< HEAD
  const importVdc = useEngineStore((s) => s.importVdc);
  const applyDdc = useEngineStore((s) => s.applyDdc);
=======
  const applyAutoEq = useEngineStore((s) => s.applyAutoEq);
>>>>>>> feat/autoeq-fetch

  const [presets, setPresets] = useState<EqPreset[]>([]);
  const [showImport, setShowImport] = useState(false);
  const [curveText, setCurveText] = useState("");
  const [importing, setImporting] = useState(false);
  const [importingVdc, setImportingVdc] = useState(false);

  // Bundled ViPER DDC preset library (600+ shipped curves) to apply in one pick.
  const [ddcNames, setDdcNames] = useState<string[]>([]);
  const [ddcLoading, setDdcLoading] = useState(false);
  const [appliedDdc, setAppliedDdc] = useState<string | null>(null);
  const [applyingDdc, setApplyingDdc] = useState(false);

  const [showAutoEq, setShowAutoEq] = useState(false);
  const [autoEqQuery, setAutoEqQuery] = useState("");
  const [autoEqResults, setAutoEqResults] = useState<AutoEqEntry[]>([]);
  const [autoEqSearching, setAutoEqSearching] = useState(false);
  const [applyingUrl, setApplyingUrl] = useState<string | null>(null);

  const refresh = useCallback(() => {
    eqListPresets()
      .then(setPresets)
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Load the bundled DDC curve names once so the picker is ready immediately
  // (one cheap call returning the names of the shipped curves).
  useEffect(() => {
    setDdcLoading(true);
    ddcList()
      .then(setDdcNames)
      .catch((e) => toast.error(`Couldn't load DDC presets: ${ipcErrorMessage(e)}`))
      .finally(() => setDdcLoading(false));
  }, []);

  const handleApply = (id: string) => {
    eqApplyPreset(id)
      .then((preset) => applyPreset(preset))
      .catch(() => {});
  };

  const handleSave = (name: string) => {
    eqSaveCustom(name, bands, preGain)
      .then((saved) => {
        refresh();
        applyPreset(saved);
      })
      .catch(() => {});
  };

  const handleDelete = (id: string) => {
    eqDelete(id)
      .then(refresh)
      .catch(() => {});
  };

  const reset = () => setBands(Array<number>(BAND_COUNT).fill(0));

  const applyCurve = async () => {
    setImporting(true);
    try {
      await importGraphicEq(curveText);
      setShowImport(false);
      setCurveText("");
      toast.success("EQ curve imported");
    } catch (e) {
      toast.error(`Couldn't import curve: ${ipcErrorMessage(e)}`);
    } finally {
      setImporting(false);
    }
  };

<<<<<<< HEAD
  const importVdcFile = async () => {
    const path = await open({
      multiple: false,
      filters: [{ name: "ViPER DDC", extensions: ["vdc"] }],
    });
    if (typeof path !== "string") return;
    setImportingVdc(true);
    try {
      await importVdc(path);
      toast.success("DDC imported to the equalizer");
    } catch (e) {
      toast.error(`Couldn't import .vdc: ${ipcErrorMessage(e)}`);
    } finally {
      setImportingVdc(false);
    }
  };

  const applyDdcPreset = async (name: string) => {
    setApplyingDdc(true);
    try {
      await applyDdc(name);
      setAppliedDdc(name);
      toast.success(`Applied ${name}`);
    } catch (e) {
      toast.error(`Couldn't apply ${name}: ${ipcErrorMessage(e)}`);
    } finally {
      setApplyingDdc(false);
=======
  // Debounced AutoEQ database search (instant + offline; index is bundled).
  // The `cancelled` flag guards against a slow in-flight response overwriting
  // results for a newer query.
  useEffect(() => {
    if (!showAutoEq) return;
    const q = autoEqQuery.trim();
    if (q === "") {
      setAutoEqResults([]);
      setAutoEqSearching(false);
      return;
    }
    setAutoEqSearching(true);
    let cancelled = false;
    const handle = setTimeout(() => {
      autoeqSearch(q, 30)
        .then((r) => { if (!cancelled) setAutoEqResults(r); })
        .catch(() => { if (!cancelled) setAutoEqResults([]); })
        .finally(() => { if (!cancelled) setAutoEqSearching(false); });
    }, 250);
    return () => { cancelled = true; clearTimeout(handle); };
  }, [autoEqQuery, showAutoEq]);

  const applyAutoEqEntry = async (entry: AutoEqEntry) => {
    setApplyingUrl(entry.url);
    try {
      await applyAutoEq(entry.url);
      toast.success(`Applied ${entry.name}`);
      setShowAutoEq(false);
      setAutoEqQuery("");
      setAutoEqResults([]);
    } catch (e) {
      toast.error(`Couldn't apply ${entry.name}: ${ipcErrorMessage(e)}`);
    } finally {
      setApplyingUrl(null);
>>>>>>> feat/autoeq-fetch
    }
  };

  return (
    <div className="mx-auto w-full max-w-5xl">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      <Card
        title="Graphic equalizer"
        icon={route.icon}
        actions={
          <label className="flex items-center gap-2 text-sm text-text-muted">
            <span>EQ</span>
            <Switch
              checked={enabled}
              onChange={setEqEnabled}
              label="Enable equalizer"
            />
          </label>
        }
      >
        <div className="flex flex-col gap-4">
          {/* One place to choose a profile: EQ presets + ViPER DDC curves. */}
          <PresetPicker
            presets={presets}
            activeId={activePresetId}
            onApply={handleApply}
            onSave={handleSave}
            onDelete={handleDelete}
            ddcNames={ddcNames}
            ddcLoading={ddcLoading}
            appliedDdc={appliedDdc}
            applyingDdc={applyingDdc}
            onApplyDdc={applyDdcPreset}
          />

          <EqVisualizer bands={bands} />

          {/* 31 band faders */}
          <div className="flex h-44 items-stretch gap-1">
            {bands.map((value, i) => (
              <BandColumn key={i} index={i} value={value} onBandChange={setBand} />
            ))}
          </div>

          {/* pre-gain + reset */}
          <div className="flex items-center justify-between gap-6 border-t border-border pt-4">
            <div className="flex flex-1 items-center gap-3">
              <span className="w-16 shrink-0 text-sm text-text-muted">
                Pre-gain
              </span>
              <Slider
                label="EQ pre-gain"
                min={-12}
                max={12}
                step={0.5}
                value={preGain}
                onChange={setPreGain}
                formatValue={(v) => `${formatDb(v)} decibels`}
                className="flex-1 max-w-xs"
              />
              <span className="w-14 text-right text-xs tabular-nums text-text-muted">
                {formatDb(preGain)} dB
              </span>
            </div>
            <Button variant="ghost" onClick={reset}>
              <RotateCcw className="size-4" aria-hidden="true" />
              Reset
            </Button>
          </div>

<<<<<<< HEAD
          {/* Advanced: import a custom curve / DDC file (picking a bundled preset
              lives in the picker above). */}
=======
          {/* AutoEQ database + manual import affordances */}
>>>>>>> feat/autoeq-fetch
          <div className="border-t border-border pt-3">
            <div className="flex flex-wrap gap-2">
              <Button
                variant="ghost"
<<<<<<< HEAD
                onClick={() => setShowImport((v) => !v)}
=======
                onClick={() => { setShowAutoEq((v) => !v); setShowImport(false); }}
                aria-expanded={showAutoEq}
              >
                Find headphone (AutoEQ)…
              </Button>
              <Button
                variant="ghost"
                onClick={() => { setShowImport((v) => !v); setShowAutoEq(false); }}
>>>>>>> feat/autoeq-fetch
                aria-expanded={showImport}
              >
                Import curve…
              </Button>
<<<<<<< HEAD
              <Button
                variant="ghost"
                onClick={importVdcFile}
                disabled={importingVdc}
              >
                {importingVdc ? "Importing…" : "Import .vdc file"}
              </Button>
            </div>

=======
            </div>

            {showAutoEq && (
              <div className="mt-3 space-y-2">
                <input
                  type="search"
                  autoFocus
                  className="w-full rounded-md bg-white/5 px-3 py-2 text-sm text-text-primary placeholder:text-text-faint focus:outline-none focus:ring-1 focus:ring-accent"
                  placeholder="Search 3,900+ headphones — e.g. Sennheiser HD 600"
                  value={autoEqQuery}
                  onChange={(e) => setAutoEqQuery(e.target.value)}
                  spellCheck={false}
                />
                <div className="max-h-56 overflow-y-auto rounded-md border border-border">
                  {autoEqQuery.trim() === "" ? (
                    <p className="px-3 py-3 text-xs text-text-faint">
                      Type a model name to find its AutoEq correction curve.
                    </p>
                  ) : autoEqResults.length === 0 ? (
                    <p className="px-3 py-3 text-xs text-text-faint">
                      {autoEqSearching ? "Searching…" : "No matching headphones."}
                    </p>
                  ) : (
                    <ul className="divide-y divide-border/60">
                      {autoEqResults.map((entry) => (
                        <li key={`${entry.source}/${entry.name}`}>
                          <button
                            type="button"
                            onClick={() => applyAutoEqEntry(entry)}
                            disabled={applyingUrl !== null}
                            className="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-sm hover:bg-white/5 disabled:opacity-50"
                          >
                            <span className="min-w-0 flex-1 truncate text-text-primary">
                              {entry.name}
                            </span>
                            <span className="shrink-0 text-[10px] uppercase tracking-wide text-text-faint">
                              {applyingUrl === entry.url ? "Applying…" : entry.source}
                            </span>
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
                <p className="text-[10px] text-text-faint">
                  Curves from the AutoEq project — fetched on demand and mapped to the 31 bands with a clip-proof preamp.
                </p>
              </div>
            )}

>>>>>>> feat/autoeq-fetch
            {showImport && (
              <div className="mt-3 space-y-2">
                <textarea
                  className="h-24 w-full rounded-md border border-border bg-surface p-2 text-xs text-text placeholder:text-text-faint transition-colors focus:border-accent focus:outline-none"
                  placeholder="GraphicEQ: 20 -1.2; 25 -1.1; ... (paste an AutoEQ curve)"
                  value={curveText}
                  onChange={(e) => setCurveText(e.target.value)}
                  spellCheck={false}
                />
                <div className="flex gap-2">
                  <Button
                    variant="primary"
                    onClick={applyCurve}
                    disabled={importing || curveText.trim() === ""}
                  >
                    {importing ? "Applying…" : "Apply curve"}
                  </Button>
                  <Button
                    variant="ghost"
                    onClick={() => { setShowImport(false); setCurveText(""); }}
                  >
                    Cancel
                  </Button>
                </div>
              </div>
            )}
          </div>
        </div>
      </Card>
    </div>
  );
}
