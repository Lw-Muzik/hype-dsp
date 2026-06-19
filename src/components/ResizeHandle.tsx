import { useCallback, useEffect, useRef, useState } from "react";
import { SIDEBAR_LIMITS, useUiStore } from "@/stores/ui";
import { cn } from "@/lib/cn";

const clamp = (v: number, min: number, max: number): number =>
  Math.min(max, Math.max(min, v));

/**
 * A draggable separator that resizes the left or right sidebar.
 *
 * `side` is the sidebar it controls. The handle straddles that sidebar's inner
 * edge: dragging the left sidebar's handle right grows it; dragging the right
 * sidebar's handle left grows it. Width commits are rAF-throttled (so a fast
 * drag doesn't thrash the virtualized lists inside the panels) and the global
 * `resizing` flag drops the sidebars' width transition so they track 1:1.
 *
 * Renders nothing when its sidebar isn't resizable (left collapsed / right
 * closed). Keyboard: ←/→ nudge (Shift = larger), Home/End jump to min/max,
 * double-click resets to the default width.
 */
export function ResizeHandle({ side }: { side: "left" | "right" }) {
  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const rightOpen = useUiStore((s) => s.rightPanel !== null);
  const width = useUiStore((s) => (side === "left" ? s.leftWidth : s.rightWidth));
  const setWidth = useUiStore((s) =>
    side === "left" ? s.setLeftWidth : s.setRightWidth,
  );
  const setResizing = useUiStore((s) => s.setResizing);

  const [active, setActive] = useState(false);
  const drag = useRef({ startX: 0, startW: 0, raf: 0, target: 0, active: false });

  const { min, max, default: defaultWidth } = SIDEBAR_LIMITS[side];
  // Left handle sits on the right edge → drag right (+dx) grows. Right handle
  // sits on the left edge → drag left (−dx) grows.
  const grows = side === "left" ? 1 : -1;

  const stopDrag = useCallback(() => {
    const d = drag.current;
    if (!d.active) return;
    d.active = false;
    if (d.raf) {
      cancelAnimationFrame(d.raf);
      d.raf = 0;
    }
    setWidth(d.target);
    setActive(false);
    setResizing(false);
    document.body.style.userSelect = "";
    document.body.style.cursor = "";
  }, [setWidth, setResizing]);

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (e.button !== 0) return;
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
      drag.current = {
        startX: e.clientX,
        startW: width,
        raf: 0,
        target: width,
        active: true,
      };
      setActive(true);
      setResizing(true);
      document.body.style.userSelect = "none";
      document.body.style.cursor = "col-resize";
    },
    [width, setResizing],
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const d = drag.current;
      if (!d.active) return;
      const next = clamp(d.startW + (e.clientX - d.startX) * grows, min, max);
      d.target = next;
      // Coalesce moves into one commit per frame.
      if (!d.raf) {
        d.raf = requestAnimationFrame(() => {
          d.raf = 0;
          setWidth(d.target);
        });
      }
    },
    [grows, min, max, setWidth],
  );

  // If the handle unmounts mid-drag (e.g. the panel closes), don't leave the
  // body locked into the resize cursor / no-select.
  useEffect(
    () => () => {
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
    },
    [],
  );

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      const step = (e.shiftKey ? 24 : 8) * grows;
      if (e.key === "ArrowRight") setWidth(width + step);
      else if (e.key === "ArrowLeft") setWidth(width - step);
      else if (e.key === "Home") setWidth(grows === 1 ? min : max);
      else if (e.key === "End") setWidth(grows === 1 ? max : min);
      else return;
      e.preventDefault();
    },
    [grows, min, max, width, setWidth],
  );

  if (side === "left" ? collapsed : !rightOpen) return null;

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={side === "left" ? "Resize navigation sidebar" : "Resize panel"}
      aria-valuenow={Math.round(width)}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={stopDrag}
      onPointerCancel={stopDrag}
      onLostPointerCapture={stopDrag}
      onDoubleClick={() => setWidth(defaultWidth)}
      onKeyDown={onKeyDown}
      className="group relative z-20 flex w-1.5 shrink-0 cursor-col-resize touch-none select-none items-stretch justify-center focus:outline-none"
    >
      <span
        className={cn(
          "h-full w-px transition-colors",
          active
            ? "w-0.5 bg-accent"
            : "bg-border group-hover:bg-accent/60 group-focus-visible:bg-accent/60",
        )}
      />
    </div>
  );
}
