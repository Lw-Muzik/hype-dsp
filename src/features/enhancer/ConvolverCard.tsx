import { useState } from "react";
import { Waves } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { Card } from "@/components/Card";
import { Switch } from "@/components/Switch";
import { Slider } from "@/components/Slider";
import { useEngineStore } from "@/stores/engine";
import { cn } from "@/lib/cn";

const pct = (v: number) => `${Math.round(v * 100)}%`;
const db = (v: number) => `${v >= 0 ? "+" : ""}${v.toFixed(1)} dB`;

/** Convolution (impulse-response) reverb / room correction. */
export function ConvolverCard() {
  const convolver = useEngineStore((s) => s.state.convolver);
  const setConvolver = useEngineStore((s) => s.setConvolver);
  const loadConvolverIr = useEngineStore((s) => s.loadConvolverIr);
  const [loading, setLoading] = useState(false);

  const pickIr = async () => {
    const path = await open({
      multiple: false,
      filters: [{ name: "Impulse response", extensions: ["wav", "irs"] }],
    });
    if (typeof path !== "string") return;
    setLoading(true);
    try {
      await loadConvolverIr(path);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Card
      title="Convolver"
      icon={Waves}
      actions={
        <Switch
          checked={convolver.enabled}
          onChange={(v) => setConvolver({ ...convolver, enabled: v })}
          label="Enable convolver"
        />
      }
    >
      <div className={cn("flex flex-col gap-4", !convolver.enabled && "opacity-60")}>
        {/* IR picker */}
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={pickIr}
            disabled={loading}
            className="rounded-md bg-white/5 px-3 py-1.5 text-sm hover:bg-white/10 disabled:opacity-50"
          >
            {loading ? "Loading…" : "Load IR…"}
          </button>
          <span className="truncate text-sm text-text-muted">
            {convolver.irName ?? "No impulse response loaded"}
          </span>
        </div>

        {/* Truncation notice */}
        {convolver.irTruncated && (
          <p className="text-xs text-amber-400/80">
            IR truncated to {convolver.irSeconds.toFixed(1)} s for performance.
          </p>
        )}

        {/* Mix slider */}
        <div className="flex items-center gap-3">
          <span className="w-20 shrink-0 text-sm text-text-muted">Mix</span>
          <Slider
            label="Mix"
            min={0}
            max={1}
            step={0.01}
            value={convolver.wetDry}
            onChange={(v) => setConvolver({ ...convolver, wetDry: v })}
            formatValue={pct}
            className="flex-1"
          />
          <span className="w-12 text-right text-xs tabular-nums text-text-muted">
            {pct(convolver.wetDry)}
          </span>
        </div>

        {/* IR gain slider */}
        <div className="flex items-center gap-3">
          <span className="w-20 shrink-0 text-sm text-text-muted">IR gain</span>
          <Slider
            label="IR gain"
            min={-24}
            max={24}
            step={0.5}
            value={convolver.irGainDb}
            onChange={(v) => setConvolver({ ...convolver, irGainDb: v })}
            formatValue={db}
            className="flex-1"
          />
          <span className="w-12 text-right text-xs tabular-nums text-text-muted">
            {db(convolver.irGainDb)}
          </span>
        </div>
      </div>
    </Card>
  );
}
