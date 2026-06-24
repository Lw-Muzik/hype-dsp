import { useCallback, useEffect, useState } from "react";
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
import { PresetBar } from "@/features/equalizer/PresetBar";
import { useEngineStore } from "@/stores/engine";
import { eqApplyPreset, eqDelete, eqListPresets, eqSaveCustom, ipcErrorMessage, ddcList } from "@/lib/ipc";
import { BAND_COUNT, ISO_CENTERS_HZ } from "@/lib/types";
import type { EqPreset } from "@/lib/types";
import { formatDb, formatHz } from "@/lib/format";
import { toast } from "@/stores/toast";

const DB_MIN = -12;
const DB_MAX = 12;

/** Cap rendered library rows for perf; the rest are reachable via search. */
const LIBRARY_VISIBLE = 200;

export function EqualizerView() {
  const route = routeById("equalizer");

  const bands = useEngineStore((s) => s.state.eq.bands);
  const preGain = useEngineStore((s) => s.state.eq.preGain);
  const enabled = useEngineStore((s) => s.state.eq.enabled);
  const activePresetId = useEngineStore((s) => s.state.activePresetId);
  const spectrum = useEngineStore((s) => s.spectrum);
  const setBand = useEngineStore((s) => s.setBand);
  const setBands = useEngineStore((s) => s.setBands);
  const setPreGain = useEngineStore((s) => s.setPreGain);
  const setEqEnabled = useEngineStore((s) => s.setEqEnabled);
  const applyPreset = useEngineStore((s) => s.applyPreset);
  const importGraphicEq = useEngineStore((s) => s.importGraphicEq);
  const importVdc = useEngineStore((s) => s.importVdc);
  const applyDdc = useEngineStore((s) => s.applyDdc);

  const [presets, setPresets] = useState<EqPreset[]>([]);
  const [showImport, setShowImport] = useState(false);
  const [curveText, setCurveText] = useState("");
  const [importing, setImporting] = useState(false);
  const [importingVdc, setImportingVdc] = useState(false);

  // Bundled ViPER DDC preset library (600+ shipped curves) to apply in one click.
  const [showLibrary, setShowLibrary] = useState(false);
  const [ddcNames, setDdcNames] = useState<string[]>([]);
  const [librarySearch, setLibrarySearch] = useState("");
  const [libraryLoading, setLibraryLoading] = useState(false);
  const [applyingName, setApplyingName] = useState<string | null>(null);

  const refresh = useCallback(() => {
    eqListPresets()
      .then(setPresets)
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

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

  // Load the bundled DDC preset names once, when the library panel first opens.
  useEffect(() => {
    if (showLibrary && ddcNames.length === 0 && !libraryLoading) {
      setLibraryLoading(true);
      ddcList()
        .then(setDdcNames)
        .catch((e) => toast.error(`Couldn't load DDC presets: ${ipcErrorMessage(e)}`))
        .finally(() => setLibraryLoading(false));
    }
  }, [showLibrary, ddcNames.length, libraryLoading]);

  const applyDdcPreset = async (name: string) => {
    setApplyingName(name);
    try {
      await applyDdc(name);
      toast.success(`Applied ${name}`);
    } catch (e) {
      toast.error(`Couldn't apply ${name}: ${ipcErrorMessage(e)}`);
    } finally {
      setApplyingName(null);
    }
  };

  const libraryMatches = ddcNames.filter((n) =>
    n.toLowerCase().includes(librarySearch.trim().toLowerCase()),
  );

  return (
    <div className="mx-auto w-full max-w-5xl">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      <Card title="Graphic equalizer" icon={route.icon}>
        <div className="flex flex-col gap-4">
          <div className="flex items-center justify-between gap-4">
            <div className="min-w-0 flex-1">
              <PresetBar
                presets={presets}
                activeId={activePresetId}
                onApply={handleApply}
                onSave={handleSave}
                onDelete={handleDelete}
              />
            </div>
            <div className="flex shrink-0 items-center gap-2 text-sm text-text-muted">
              <span>EQ</span>
              <Switch
                checked={enabled}
                onChange={setEqEnabled}
                label="Enable equalizer"
              />
            </div>
          </div>

          <EqVisualizer bands={bands} spectrum={spectrum} />

          {/* 31 band faders */}
          <div className="flex h-44 items-stretch gap-1">
            {bands.map((value, i) => (
              <div key={i} className="flex flex-1 flex-col items-center gap-1">
                <EqBandSlider
                  value={value}
                  min={DB_MIN}
                  max={DB_MAX}
                  label={formatHz(ISO_CENTERS_HZ[i] ?? 20)}
                  onChange={(v) => setBand(i, v)}
                />
                <span className="h-3 text-[8px] leading-none text-text-faint">
                  {i % 3 === 0 ? formatHz(ISO_CENTERS_HZ[i] ?? 20) : ""}
                </span>
              </div>
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

          {/* Import affordances */}
          <div className="border-t border-border pt-3">
            <div className="flex flex-wrap gap-2">
              <Button
                variant="ghost"
                onClick={() => setShowImport((v) => !v)}
                aria-expanded={showImport}
              >
                Import curve…
              </Button>
              <Button
                variant="ghost"
                onClick={importVdcFile}
                disabled={importingVdc}
              >
                {importingVdc ? "Importing…" : "Import .vdc file"}
              </Button>
              <Button
                variant="ghost"
                onClick={() => setShowLibrary((v) => !v)}
                aria-expanded={showLibrary}
              >
                DDC presets…
              </Button>
            </div>

            {showLibrary && (
              <div className="mt-3 space-y-2">
                <input
                  type="search"
                  autoFocus
                  className="w-full rounded-md bg-white/5 px-3 py-2 text-sm text-text-primary placeholder:text-text-faint focus:outline-none focus:ring-1 focus:ring-accent"
                  placeholder={`Search ${ddcNames.length || ""} bundled ViPER DDC presets…`}
                  value={librarySearch}
                  onChange={(e) => setLibrarySearch(e.target.value)}
                  spellCheck={false}
                />
                <div className="max-h-64 overflow-y-auto rounded-md border border-border">
                  {libraryLoading ? (
                    <p className="px-3 py-3 text-xs text-text-faint">Loading presets…</p>
                  ) : libraryMatches.length === 0 ? (
                    <p className="px-3 py-3 text-xs text-text-faint">
                      {ddcNames.length === 0
                        ? "No bundled presets found."
                        : "No presets match your search."}
                    </p>
                  ) : (
                    <ul className="divide-y divide-border/60">
                      {libraryMatches.slice(0, LIBRARY_VISIBLE).map((name) => (
                        <li key={name}>
                          <button
                            type="button"
                            onClick={() => applyDdcPreset(name)}
                            disabled={applyingName !== null}
                            className="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-sm hover:bg-white/5 disabled:opacity-50"
                          >
                            <span className="min-w-0 flex-1 truncate text-text-primary">
                              {name}
                            </span>
                            {applyingName === name && (
                              <span className="shrink-0 text-[10px] uppercase tracking-wide text-text-faint">
                                Applying…
                              </span>
                            )}
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
                {libraryMatches.length > LIBRARY_VISIBLE && (
                  <p className="text-[10px] text-text-faint">
                    Showing {LIBRARY_VISIBLE} of {libraryMatches.length} — refine your search to see the rest.
                  </p>
                )}
              </div>
            )}
            {showImport && (
              <div className="mt-3 space-y-2">
                <textarea
                  className="h-24 w-full rounded-md bg-white/5 p-2 text-xs text-text-primary placeholder:text-text-faint focus:outline-none focus:ring-1 focus:ring-accent"
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
