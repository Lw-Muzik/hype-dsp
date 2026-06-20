import { AudioLines } from "lucide-react";
import { useUiStore } from "@/stores/ui";
import { cn } from "@/lib/cn";

/**
 * Jumps to the embedded MilkDrop visualizer (the Visuals view in the middle
 * section). Always available — the embedded renderer runs in the webview; the
 * fullscreen pop-out is offered from inside that view.
 */
export function VisualizerButton({ className }: { className?: string }) {
  const route = useUiStore((s) => s.route);
  const setRoute = useUiStore((s) => s.setRoute);
  const active = route === "visuals";

  return (
    <button
      type="button"
      aria-label="Open visualizer"
      aria-pressed={active}
      onClick={() => setRoute("visuals")}
      title="Visualizer"
      className={cn(
        "flex size-8 items-center justify-center rounded-full text-text-muted transition-colors hover:text-text",
        active && "text-accent hover:text-accent",
        className,
      )}
    >
      <AudioLines className="size-4" aria-hidden="true" />
    </button>
  );
}
