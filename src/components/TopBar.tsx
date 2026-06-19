import { Volume2 } from "lucide-react";
import { routeById } from "@/app/routes";
import { useUiStore } from "@/stores/ui";
import { useEngineStore } from "@/stores/engine";
import { PowerToggle } from "@/components/PowerToggle";
import { Slider } from "@/components/Slider";
import { LevelMeter } from "@/components/LevelMeter";

/**
 * Top bar: current view, master volume, live output meters, and the global
 * power switch. Master volume and power are wired to the engine store; in
 * Phase 0 those are local state, becoming IPC-backed in Phase 2.
 */
export function TopBar() {
  const route = useUiStore((s) => s.route);
  const current = routeById(route);
  const Icon = current.icon;

  const power = useEngineStore((s) => s.state.power);
  const setPower = useEngineStore((s) => s.setPower);
  const masterVolume = useEngineStore((s) => s.state.masterVolume);
  const setMasterVolume = useEngineStore((s) => s.setMasterVolume);

  const volumePct = Math.round(masterVolume * 100);

  return (
    <header className="flex h-14 shrink-0 items-center gap-4 border-b border-border bg-surface px-5">
      <div className="flex min-w-0 items-center gap-2">
        <Icon className="size-4 text-text-muted" aria-hidden="true" />
        <h1 className="truncate text-sm font-medium">{current.label}</h1>
      </div>

      <div className="ml-auto flex items-center gap-5">
        <div className="flex items-center gap-2.5">
          <Volume2 className="size-4 text-text-muted" aria-hidden="true" />
          <Slider
            label="Master volume"
            min={0}
            max={2}
            step={0.01}
            value={masterVolume}
            onChange={setMasterVolume}
            formatValue={(v) => `${Math.round(v * 100)} percent`}
            className="w-32"
          />
          <span className="w-10 text-right text-xs tabular-nums text-text-muted">
            {volumePct}%
          </span>
        </div>

        <div className="hidden sm:block">
          <LevelMeter />
        </div>

        <PowerToggle on={power} onToggle={setPower} />
      </div>
    </header>
  );
}
