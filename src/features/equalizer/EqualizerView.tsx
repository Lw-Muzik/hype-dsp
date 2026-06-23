import { useCallback, useEffect, useState } from "react";
import { RotateCcw } from "lucide-react";
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
import { eqApplyPreset, eqDelete, eqListPresets, eqSaveCustom, ipcErrorMessage } from "@/lib/ipc";
import { BAND_COUNT, ISO_CENTERS_HZ } from "@/lib/types";
import type { EqPreset } from "@/lib/types";
import { formatDb, formatHz } from "@/lib/format";
import { toast } from "@/stores/toast";

const DB_MIN = -12;
const DB_MAX = 12;

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

  const [presets, setPresets] = useState<EqPreset[]>([]);
  const [showImport, setShowImport] = useState(false);
  const [curveText, setCurveText] = useState("");
  const [importing, setImporting] = useState(false);

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

          {/* Import curve affordance */}
          <div className="border-t border-border pt-3">
            <Button
              variant="ghost"
              onClick={() => setShowImport((v) => !v)}
            >
              Import curve…
            </Button>
            {showImport && (
              <div className="mt-3 space-y-2">
                <textarea
                  className="h-24 w-full rounded-md bg-white/5 p-2 text-xs text-text-primary placeholder:text-text-faint focus:outline-none focus:ring-1 focus:ring-accent"
                  placeholder="GraphicEQ: 20 -1.2; 25 -1.1; ... (paste an AutoEQ curve)"
                  value={curveText}
                  onChange={(e) => setCurveText(e.target.value)}
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
