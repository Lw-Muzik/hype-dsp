import { Gauge } from "lucide-react";
import { Card } from "@/components/Card";
import { Switch } from "@/components/Switch";
import { Slider } from "@/components/Slider";
import { useEngineStore } from "@/stores/engine";
import { cn } from "@/lib/cn";
import type { CompanderState } from "@/lib/types";

type Key =
  | "thresholdDb"
  | "ratio"
  | "kneeDb"
  | "attackMs"
  | "releaseMs"
  | "makeupDb"
  | "gateDb"
  | "expanderRatio";

interface Def {
  key: Key;
  label: string;
  min: number;
  max: number;
  step: number;
  fmt: (v: number) => string;
}

const db = (v: number) => `${v.toFixed(1)} dB`;
const ms = (v: number) => `${Math.round(v)} ms`;
const x = (v: number) => `${v.toFixed(1)}:1`;

const SLIDERS: readonly Def[] = [
  { key: "thresholdDb", label: "Threshold", min: -60, max: 0, step: 0.5, fmt: db },
  { key: "ratio", label: "Ratio", min: 1, max: 20, step: 0.1, fmt: x },
  { key: "kneeDb", label: "Knee", min: 0, max: 24, step: 0.5, fmt: db },
  { key: "attackMs", label: "Attack", min: 1, max: 200, step: 1, fmt: ms },
  { key: "releaseMs", label: "Release", min: 10, max: 1000, step: 5, fmt: ms },
  { key: "makeupDb", label: "Makeup", min: 0, max: 24, step: 0.5, fmt: db },
  { key: "gateDb", label: "Gate", min: -90, max: -20, step: 1, fmt: db },
  { key: "expanderRatio", label: "Expander", min: 1, max: 10, step: 0.1, fmt: x },
];

const NIGHT: Partial<CompanderState> = {
  thresholdDb: -30,
  ratio: 6,
  makeupDb: 6,
  enabled: true,
};

const PUNCH: Partial<CompanderState> = {
  ratio: 1.5,
  expanderRatio: 3,
  enabled: true,
};

export function CompanderCard() {
  const c = useEngineStore((s) => s.state.compander);
  const setCompander = useEngineStore((s) => s.setCompander);

  return (
    <Card
      title="Multiband compander"
      icon={Gauge}
      actions={
        <Switch
          checked={c.enabled}
          onChange={(enabled) => setCompander({ ...c, enabled })}
          label="Enable multiband compander"
        />
      }
    >
      <div className={cn("flex flex-col gap-3", !c.enabled && "opacity-60")}>
        {SLIDERS.map((d) => (
          <div key={d.key} className="flex items-center gap-3">
            <span className="w-20 shrink-0 text-sm text-text-muted">{d.label}</span>
            <Slider
              label={d.label}
              className="flex-1"
              min={d.min}
              max={d.max}
              step={d.step}
              value={c[d.key]}
              formatValue={d.fmt}
              onChange={(v) => setCompander({ ...c, [d.key]: v })}
            />
            <span className="w-16 text-right text-xs tabular-nums text-text-muted">
              {d.fmt(c[d.key])}
            </span>
          </div>
        ))}

        <div className="mt-1 flex gap-2">
          <button
            type="button"
            onClick={() => setCompander({ ...c, ...NIGHT })}
            className="rounded-control border border-border px-3 py-1.5 text-sm text-text-muted transition-colors hover:text-text"
          >
            Night mode
          </button>
          <button
            type="button"
            onClick={() => setCompander({ ...c, ...PUNCH })}
            className="rounded-control border border-border px-3 py-1.5 text-sm text-text-muted transition-colors hover:text-text"
          >
            Punch
          </button>
        </div>
      </div>
    </Card>
  );
}
