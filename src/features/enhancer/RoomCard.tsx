import { Building2 } from "lucide-react";
import { Card } from "@/components/Card";
import { Switch } from "@/components/Switch";
import { Slider } from "@/components/Slider";
import { Combobox } from "@/components/Combobox";
import type { ComboItem } from "@/components/Combobox";
import { useEngineStore } from "@/stores/engine";
import { ROOM_PRESETS } from "@/lib/types";
import { cn } from "@/lib/cn";

/** The six continuous room-reverb parameters. */
type RoomParam = "roomSize" | "decay" | "damping" | "preDelay" | "diffusion" | "wetDry";

interface SliderDef {
  key: RoomParam;
  label: string;
  min: number;
  max: number;
  step: number;
  format: (v: number) => string;
}

const pct = (v: number) => `${Math.round(v * 100)}%`;

const SLIDERS: readonly SliderDef[] = [
  { key: "roomSize", label: "Room size", min: 0, max: 1, step: 0.01, format: pct },
  { key: "decay", label: "Decay", min: 0, max: 1, step: 0.01, format: pct },
  { key: "damping", label: "Damping", min: 0, max: 1, step: 0.01, format: pct },
  { key: "preDelay", label: "Pre-delay", min: 0, max: 200, step: 1, format: (v) => `${Math.round(v)} ms` },
  { key: "diffusion", label: "Diffusion", min: 0, max: 1, step: 0.01, format: pct },
  { key: "wetDry", label: "Mix", min: 0, max: 1, step: 0.01, format: pct },
];

const PRESET_ITEMS: ComboItem[] = ROOM_PRESETS.map((p) => ({ id: p.id, label: p.name }));

/** Room reverb ("room effects") — Freeverb with presets, ported from mobile. */
export function RoomCard() {
  const room = useEngineStore((s) => s.state.room);
  const setRoom = useEngineStore((s) => s.setRoom);

  const applyPreset = (id: string) => {
    const p = ROOM_PRESETS.find((x) => x.id === id);
    if (!p) return;
    setRoom({
      enabled: true,
      roomSize: p.roomSize,
      decay: p.decay,
      damping: p.damping,
      preDelay: p.preDelay,
      diffusion: p.diffusion,
      wetDry: p.wetDry,
      activePresetId: p.id,
    });
  };

  return (
    <Card
      title="Room reverb"
      icon={Building2}
      actions={
        <Switch
          checked={room.enabled}
          onChange={(v) => setRoom({ ...room, enabled: v })}
          label="Enable room reverb"
        />
      }
    >
      <div className={cn("flex flex-col gap-4", !room.enabled && "opacity-60")}>
        <Combobox
          items={PRESET_ITEMS}
          value={room.activePresetId}
          onSelect={applyPreset}
          onClear={() => setRoom({ ...room, activePresetId: null })}
          placeholder="Choose a room preset…"
          searchPlaceholder="Search presets…"
          emptyText="No matching preset"
        />
        <div className="grid gap-x-6 gap-y-3 sm:grid-cols-2">
          {SLIDERS.map((s) => (
            <div key={s.key} className="flex items-center gap-3">
              <span className="w-20 shrink-0 text-sm text-text-muted">{s.label}</span>
              <Slider
                label={s.label}
                min={s.min}
                max={s.max}
                step={s.step}
                value={room[s.key]}
                onChange={(v) =>
                  setRoom({ ...room, [s.key]: v, activePresetId: null })
                }
                formatValue={s.format}
                className="flex-1"
              />
              <span className="w-12 text-right text-xs tabular-nums text-text-muted">
                {s.format(room[s.key])}
              </span>
            </div>
          ))}
        </div>
      </div>
    </Card>
  );
}
