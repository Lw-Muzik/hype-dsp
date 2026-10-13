import { useEffect, useRef, useState } from "react";
import { Cast, Globe, Laptop, Smartphone } from "lucide-react";
import { barIconBtn } from "@/components/BarParts";
import { cn } from "@/lib/cn";
import { useConnectStore } from "../../store";
import type { DeviceKind } from "../../protocol";
import { playOnDevice } from "../control";
import { useRemoteStore } from "../store";

const KIND_ICON: Record<DeviceKind, typeof Smartphone> = {
  mobile: Smartphone,
  desktop: Laptop,
  web: Globe,
};

/** Three bars that dance while the device is making sound: green is
 *  "playing now" across the whole app. */
function PlayingBars() {
  return (
    <span className="flex h-3.5 items-end gap-0.5" aria-hidden="true">
      {[0, 1, 2].map((i) => (
        <span
          key={i}
          className="w-0.5 origin-bottom animate-pulse rounded-full bg-success"
          style={{ height: `${55 + i * 20}%`, animationDelay: `${i * 140}ms` }}
        />
      ))}
    </span>
  );
}

/**
 * "Play on a device": the Connect button in the now-playing bar. Lists the
 * account's online players (this computer first), marks the one playing, and
 * moves playback to whichever is tapped. Hidden when Hype Connect is off or
 * no other player is online.
 */
export function DevicePicker({ className }: { className?: string }) {
  const enabled = useConnectStore((s) => s.status?.enabled ?? false);
  const players = useRemoteStore((s) => s.players);
  const activeId = useRemoteStore((s) => s.activeId);
  const selfId = useRemoteStore((s) => s.selfId);
  const remote = useRemoteStore((s) => s.view !== null);
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const others = players.filter((p) => p.id !== selfId);
  if (!enabled || others.length === 0) return null;

  const ordered = [...players.filter((p) => p.id === selfId), ...others];
  const playingId = ordered.some((p) => p.id === activeId) ? activeId : selfId;

  return (
    <div ref={root} className={cn("relative", className)}>
      <button
        type="button"
        aria-label="Play on a device"
        aria-haspopup="menu"
        aria-expanded={open}
        title="Play on a device"
        onClick={() => setOpen((o) => !o)}
        className={cn(barIconBtn, remote && "text-success hover:text-success")}
      >
        <Cast className="size-4" aria-hidden="true" />
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Play on"
          className="absolute bottom-10 right-0 z-40 w-72 overflow-hidden rounded-card border border-border bg-surface-overlay shadow-xl"
        >
          <p className="px-3.5 pb-1.5 pt-3 text-xs font-medium uppercase tracking-wide text-text-faint">
            Play on
          </p>
          <ul className="px-1.5 pb-1.5">
            {ordered.map((d) => {
              const Icon = KIND_ICON[d.kind];
              const isSelf = d.id === selfId;
              const playing = d.id === playingId;
              return (
                <li key={d.id}>
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setOpen(false);
                      if (!playing) void playOnDevice(d.id);
                    }}
                    className={cn(
                      "flex w-full items-center gap-3 rounded-control px-2.5 py-2 text-left transition-colors hover:bg-surface-raised",
                      playing && "bg-surface-raised",
                    )}
                  >
                    <Icon
                      className={cn("size-5 shrink-0", playing ? "text-success" : "text-text-muted")}
                      aria-hidden="true"
                    />
                    <span className="min-w-0 flex-1">
                      <span
                        className={cn(
                          "block truncate text-sm font-medium",
                          playing && "text-success",
                        )}
                      >
                        {isSelf ? "This computer" : d.name}
                      </span>
                      <span className="block truncate text-xs text-text-faint">
                        {playing ? "Playing now" : isSelf ? d.name : (d.platform ?? "Hype Muzik")}
                      </span>
                    </span>
                    {playing && <PlayingBars />}
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}
