import { Palette } from "lucide-react";
import { Card } from "@/components/Card";
import { Slider } from "@/components/Slider";
import { Segmented } from "@/components/Segmented";
import { THEME_LIMITS, useThemeStore, type ThemeChoice } from "@/stores/theme";

const CHOICES: readonly { value: ThemeChoice; label: string }[] = [
  { value: "system", label: "System" },
  { value: "dynamic", label: "Dynamic" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

export default function ThemeCard() {
  const choice = useThemeStore((s) => s.choice);
  const resolved = useThemeStore((s) => s.resolved);
  const blur = useThemeStore((s) => s.blur);
  const setChoice = useThemeStore((s) => s.setChoice);
  const setBlur = useThemeStore((s) => s.setBlur);

  const dynamic = resolved === "dynamic";

  return (
    <Card title="Appearance" icon={Palette}>
      <div className="flex flex-col gap-4">
        <div className="flex flex-col gap-1.5">
          <Segmented items={CHOICES} value={choice} onChange={setChoice} label="Theme" />
          <p className="text-xs text-text-faint">
            {choice === "system"
              ? "Follows your system appearance."
              : choice === "dynamic"
                ? "The album art of whatever's playing, blurred behind the app."
                : "Always this theme, whatever your system is set to."}
          </p>
        </div>

        <div className="flex items-center gap-3">
          <span className="w-20 shrink-0 text-sm text-text-muted">Blur</span>
          <Slider
            label="Backdrop blur"
            min={THEME_LIMITS.blur.min}
            max={THEME_LIMITS.blur.max}
            step={THEME_LIMITS.blur.step}
            value={blur}
            disabled={!dynamic}
            onChange={setBlur}
            formatValue={(v) => `${Math.round(v)} pixels`}
            // Slider needs an explicit width class: passing className replaces
            // its flex-1 default, and a 0px track silently ignores drags.
            className="flex-1"
          />
          <span className="w-12 text-right text-xs tabular-nums text-text-muted">
            {Math.round(blur)}px
          </span>
        </div>
      </div>
    </Card>
  );
}
