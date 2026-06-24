import { Flame } from "lucide-react";
import { Card } from "@/components/Card";
import { Switch } from "@/components/Switch";
import { Slider } from "@/components/Slider";
import { useEngineStore } from "@/stores/engine";
import { cn } from "@/lib/cn";
import type { SaturationState } from "@/lib/types";

const pct = (v: number) => `${Math.round(v * 100)}%`;

const WARM: Partial<SaturationState> = { drive: 0.2, enabled: true };
const HOT: Partial<SaturationState> = { drive: 0.6, enabled: true };

export function SaturationCard() {
  const s = useEngineStore((st) => st.state.saturation);
  const setSaturation = useEngineStore((st) => st.setSaturation);

  return (
    <Card
      title="Tube saturation"
      icon={Flame}
      actions={
        <Switch
          checked={s.enabled}
          onChange={(enabled) => setSaturation({ ...s, enabled })}
          label="Enable tube saturation"
        />
      }
    >
      <div className={cn("flex flex-col gap-3", !s.enabled && "opacity-60")}>
        <div className="flex items-center gap-3">
          <span className="w-10 shrink-0 text-sm text-text-muted">Drive</span>
          <Slider
            label="Drive"
            className="flex-1"
            min={0}
            max={1}
            step={0.01}
            value={s.drive}
            formatValue={pct}
            onChange={(v) => setSaturation({ ...s, drive: v })}
          />
          <span className="w-12 text-right text-xs tabular-nums text-text-muted">
            {pct(s.drive)}
          </span>
        </div>

        <div className="flex items-center gap-3">
          <span className="w-10 shrink-0 text-sm text-text-muted">Mix</span>
          <Slider
            label="Mix"
            className="flex-1"
            min={0}
            max={1}
            step={0.01}
            value={s.mix}
            formatValue={pct}
            onChange={(v) => setSaturation({ ...s, mix: v })}
          />
          <span className="w-12 text-right text-xs tabular-nums text-text-muted">
            {pct(s.mix)}
          </span>
        </div>

        <div className="mt-1 flex gap-2">
          <button
            type="button"
            onClick={() => setSaturation({ ...s, ...WARM })}
            className="rounded-control border border-border px-3 py-1.5 text-sm text-text-muted transition-colors hover:text-text"
          >
            Warm
          </button>
          <button
            type="button"
            onClick={() => setSaturation({ ...s, ...HOT })}
            className="rounded-control border border-border px-3 py-1.5 text-sm text-text-muted transition-colors hover:text-text"
          >
            Hot
          </button>
        </div>
      </div>
    </Card>
  );
}
