import { useEffect } from "react";
import { Cast, Settings, Volume2 } from "lucide-react";
import { routeById } from "@/app/routes";
import { useUiStore } from "@/stores/ui";
import { useEngineStore } from "@/stores/engine";
import { PowerToggle } from "@/components/PowerToggle";
import { Slider } from "@/components/Slider";
import { LevelMeter } from "@/components/LevelMeter";
import { ScrollingWaveform } from "@/features/player/ScrollingWaveform";
import { cn } from "@/lib/cn";
import { TITLEBAR_INSET, isMac } from "@/lib/platform";
import { openSettings } from "@/lib/ipc";
import { UpdatePill } from "@/features/update/UpdatePill";
import { ModeSwitch } from "@/features/mode/ModeSwitch";
import { useRemoteStore } from "@/features/connect/remote/store";
import { useRemoteVolume } from "@/features/connect/remote/ui/useRemoteVolume";

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

  // While another device is the player, the volume group steers that device
  // (0..1, no boost) instead of this computer's silent output.
  const remote = useRemoteStore((s) => s.view);
  const remoteVolume = useRemoteVolume(remote?.volume ?? 0);
  const volume = remote ? remoteVolume.value : masterVolume;
  const volumePct = Math.round(volume * 100);
  const volumeLabel = remote ? `Volume on ${remote.device.name}` : "Master volume";
  const VolumeIcon = remote ? Cast : Volume2;

  // Cmd/Ctrl+, opens Settings. The app menu carries the same accelerator; opening
  // twice just focuses the window, so the two paths can't conflict.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "," && (isMac ? e.metaKey : e.ctrlKey) && !e.altKey && !e.shiftKey) {
        e.preventDefault();
        void openSettings().catch(() => {});
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    // `data-tauri-drag-region="deep"` makes the whole bar a window drag handle;
    // interactive children (volume slider, power toggle) still receive clicks.
    // The top inset keeps content clear of the macOS traffic lights and aligns
    // the bar's height with the sidebar's brand header (see platform.ts).
    <header
      data-tauri-drag-region="deep"
      style={{ height: 56 + TITLEBAR_INSET, paddingTop: TITLEBAR_INSET }}
      className="flex shrink-0 items-center gap-4 border-b border-border bg-surface px-5"
    >
      <div className="flex shrink-0 items-center gap-2">
        <Icon className="size-4 text-text-muted" aria-hidden="true" />
        <h1 className="truncate text-sm font-medium">{current.label}</h1>
      </div>

      <ModeSwitch />

      {/* Live scrolling stereo waveform fills the space between title and
          controls (renders nothing when no track is playing). */}
      <div className="hidden h-9 min-w-0 flex-1 sm:block">
        <ScrollingWaveform />
      </div>

      <div className="ml-auto flex shrink-0 items-center gap-5">
        <UpdatePill />
        <div className="flex items-center gap-2.5">
          <span title={volumeLabel} className="grid shrink-0 place-items-center">
            <VolumeIcon
              className={cn("size-4", remote ? "text-accent" : "text-text-muted")}
              aria-hidden="true"
            />
          </span>
          <Slider
            label={volumeLabel}
            min={0}
            max={remote ? 1 : 2}
            step={0.01}
            value={volume}
            onChange={remote ? remoteVolume.onChange : setMasterVolume}
            onCommit={remote ? remoteVolume.onCommit : undefined}
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

        <button
          type="button"
          aria-label="Settings"
          title={isMac ? "Settings (⌘,)" : "Settings (Ctrl+,)"}
          onClick={() => void openSettings().catch(() => {})}
          className="grid size-8 place-items-center rounded-control text-text-faint transition-colors hover:bg-surface-overlay hover:text-text"
        >
          <Settings className="size-4" aria-hidden="true" />
        </button>
      </div>
    </header>
  );
}
