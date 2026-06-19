import { useEffect, useState } from "react";
import { Sparkles } from "lucide-react";
import {
  visualizerAvailable,
  visualizerStart,
  visualizerStop,
} from "@/lib/ipc";
import { cn } from "@/lib/cn";

/**
 * Toggles the native MilkDrop visualizer window (a separate sidecar process fed
 * the engine's audio). Renders nothing when the sidecar isn't bundled in this
 * build, so it's invisible on platforms/builds without it.
 */
export function VisualizerButton({ className }: { className?: string }) {
  const [available, setAvailable] = useState(false);
  const [on, setOn] = useState(false);

  useEffect(() => {
    visualizerAvailable()
      .then(setAvailable)
      .catch(() => setAvailable(false));
  }, []);

  if (!available) return null;

  const toggle = () => {
    if (on) {
      void visualizerStop().catch(() => {});
      setOn(false);
    } else {
      void visualizerStart()
        .then(() => setOn(true))
        .catch(() => setOn(false));
    }
  };

  return (
    <button
      type="button"
      aria-label={on ? "Close visualizer" : "Open MilkDrop visualizer"}
      aria-pressed={on}
      onClick={toggle}
      title="MilkDrop visualizer"
      className={cn(
        "flex size-8 items-center justify-center rounded-full text-text-muted transition-colors hover:text-text",
        on && "text-accent hover:text-accent",
        className,
      )}
    >
      <Sparkles className="size-4" aria-hidden="true" />
    </button>
  );
}
