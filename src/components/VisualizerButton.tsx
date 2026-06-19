import { useEffect } from "react";
import { AudioLines } from "lucide-react";
import { useVisualizerStore } from "@/stores/visualizer";
import { cn } from "@/lib/cn";

/**
 * Toggles the native MilkDrop visualizer window (a separate sidecar process fed
 * the engine's audio). Renders nothing when the sidecar isn't bundled in this
 * build, so it's invisible on platforms/builds without it. Running state and
 * render settings live in the shared visualizer store, so this stays in sync
 * with the Settings controls.
 */
export function VisualizerButton({ className }: { className?: string }) {
  const available = useVisualizerStore((s) => s.available);
  const running = useVisualizerStore((s) => s.running);
  const probe = useVisualizerStore((s) => s.probe);
  const toggle = useVisualizerStore((s) => s.toggle);

  useEffect(() => {
    probe();
  }, [probe]);

  if (!available) return null;

  return (
    <button
      type="button"
      aria-label={running ? "Close visualizer" : "Open MilkDrop visualizer"}
      aria-pressed={running}
      onClick={toggle}
      title="MilkDrop visualizer"
      className={cn(
        "flex size-8 items-center justify-center rounded-full text-text-muted transition-colors hover:text-text",
        running && "text-accent hover:text-accent",
        className,
      )}
    >
      <AudioLines className="size-4" aria-hidden="true" />
    </button>
  );
}
