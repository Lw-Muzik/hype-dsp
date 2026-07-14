import { useCallback, useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent, ReactNode } from "react";
import { createPortal } from "react-dom";
import type HlsType from "hls.js";
import {
  AlertTriangle,
  Loader2,
  Maximize,
  Maximize2,
  Minimize,
  Minimize2,
  Pause,
  Play,
  RotateCw,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import { tvStreamUrl } from "@/lib/ipc";
import { useEngineStore } from "@/stores/engine";
import type { TvChannel } from "@/lib/types";
import { cn } from "@/lib/cn";

type Status = "loading" | "playing" | "paused" | "error";
/** normal = docked PiP, mini = small PiP, full = covers the app window. */
type Mode = "normal" | "mini" | "full";

const WIDTHS: Record<Exclude<Mode, "full">, number> = { normal: 448, mini: 288 };
const MARGIN = 24;

/**
 * In-app TV player. Plays a channel in an embedded `<video>` (hls.js, native-HLS
 * fallback) fed by the local HLS proxy — no native window, no external player.
 *
 * It renders through a portal to `document.body` so it floats above the whole app
 * as a draggable picture-in-picture: the channel list stays fully browsable and
 * selecting another channel just re-points the same player (playback never
 * remounts). Fullscreen covers the app window (a portalled `fixed` overlay, not
 * the flaky webview Fullscreen API). Starting a channel stops the audio engine.
 */
export function TvPlayer({ channel, onClose }: { channel: TvChannel; onClose: () => void }) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const hlsRef = useRef<HlsType | null>(null);
  const hideTimer = useRef<number | null>(null);
  const drag = useRef<{ px: number; py: number; ox: number; oy: number } | null>(null);

  const [status, setStatus] = useState<Status>("loading");
  const [muted, setMuted] = useState(false);
  const [volume, setVolume] = useState(1);
  const [mode, setMode] = useState<Mode>("normal");
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);
  const [controlsShown, setControlsShown] = useState(true);
  const [reloadKey, setReloadKey] = useState(0);

  const stopEngine = useEngineStore((s) => s.stop);

  // Load / switch the stream whenever the channel (or a manual retry) changes.
  // The <video> element is stable across mode changes (portal), so switching
  // channels swaps the source in place without interrupting the player chrome.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    let cancelled = false;
    setStatus("loading");
    void stopEngine();

    const teardown = () => {
      if (hlsRef.current) {
        hlsRef.current.destroy();
        hlsRef.current = null;
      }
      video.removeAttribute("src");
      video.load();
    };
    teardown();

    const play = () => {
      video.play().catch(() => {
        video.muted = true;
        setMuted(true);
        void video.play().catch(() => {});
      });
    };

    tvStreamUrl(channel)
      .then(async (url) => {
        if (cancelled) return;
        const { default: Hls } = await import("hls.js");
        if (cancelled) return;
        if (Hls.isSupported()) {
          const hls = new Hls({
            enableWorker: true,
            // Fast start: prefetch the first fragment while the manifest is still
            // parsing, and don't chase the live edge (which stalls on regular,
            // non-low-latency IPTV). Default `testBandwidth` already loads the
            // first fragment at the lowest quality for a quick first frame.
            startFragPrefetch: true,
            lowLatencyMode: false,
            backBufferLength: 30,
            // Give up on stuck manifests/levels quickly so the retry/error path
            // kicks in instead of an endless spinner.
            manifestLoadingTimeOut: 8000,
            manifestLoadingMaxRetry: 3,
            levelLoadingTimeOut: 8000,
            fragLoadingTimeOut: 20000,
          });
          hlsRef.current = hls;
          hls.loadSource(url);
          hls.attachMedia(video);
          hls.on(Hls.Events.MANIFEST_PARSED, play);
          hls.on(Hls.Events.ERROR, (_e, data) => {
            if (!data.fatal) return;
            if (data.type === Hls.ErrorTypes.NETWORK_ERROR) hls.startLoad();
            else if (data.type === Hls.ErrorTypes.MEDIA_ERROR) hls.recoverMediaError();
            else setStatus("error");
          });
        } else if (video.canPlayType("application/vnd.apple.mpegurl")) {
          video.src = url;
          video.addEventListener("loadedmetadata", play, { once: true });
        } else {
          setStatus("error");
        }
      })
      .catch(() => {
        if (!cancelled) setStatus("error");
      });

    return () => {
      cancelled = true;
      teardown();
    };
  }, [channel, reloadKey, stopEngine]);

  // Reflect the media element's state in the UI.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const onPlaying = () => setStatus("playing");
    const onWaiting = () => setStatus((s) => (s === "error" ? s : "loading"));
    const onPause = () => setStatus((s) => (s === "error" ? s : "paused"));
    const onError = () => setStatus("error");
    const onVolume = () => {
      setMuted(video.muted);
      setVolume(video.volume);
    };
    video.addEventListener("playing", onPlaying);
    video.addEventListener("waiting", onWaiting);
    video.addEventListener("pause", onPause);
    video.addEventListener("error", onError);
    video.addEventListener("volumechange", onVolume);
    return () => {
      video.removeEventListener("playing", onPlaying);
      video.removeEventListener("waiting", onWaiting);
      video.removeEventListener("pause", onPause);
      video.removeEventListener("error", onError);
      video.removeEventListener("volumechange", onVolume);
    };
  }, []);

  // Escape leaves fullscreen (falls back to the docked view).
  useEffect(() => {
    if (mode !== "full") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMode("normal");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [mode]);

  // Keep the floating player within the viewport when the window resizes.
  useEffect(() => {
    const onResize = () => setPos((p) => (p ? clamp(p, mode) : p));
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [mode]);

  useEffect(() => () => {
    if (hideTimer.current) window.clearTimeout(hideTimer.current);
  }, []);

  const nudgeControls = useCallback(() => {
    setControlsShown(true);
    if (hideTimer.current) window.clearTimeout(hideTimer.current);
    hideTimer.current = window.setTimeout(() => {
      if (!videoRef.current?.paused) setControlsShown(false);
    }, 2600);
  }, []);

  const togglePlay = () => {
    const v = videoRef.current;
    if (!v) return;
    if (v.paused) void v.play().catch(() => {});
    else v.pause();
  };
  const toggleMute = () => {
    const v = videoRef.current;
    if (v) v.muted = !v.muted;
  };
  const changeVolume = (value: number) => {
    const v = videoRef.current;
    if (!v) return;
    v.volume = value;
    v.muted = value === 0;
  };

  // --- Drag (floating modes only) -----------------------------------------
  const effPos = mode === "full" ? null : (pos ?? defaultPos(mode));

  const onDragDown = (e: ReactPointerEvent) => {
    if (mode === "full" || !effPos) return;
    drag.current = { px: e.clientX, py: e.clientY, ox: effPos.x, oy: effPos.y };
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
  };
  const onDragMove = (e: ReactPointerEvent) => {
    const d = drag.current;
    if (!d) return;
    setPos(clamp({ x: d.ox + (e.clientX - d.px), y: d.oy + (e.clientY - d.py) }, mode));
  };
  const onDragUp = () => {
    drag.current = null;
  };

  const busy = status === "loading";
  const floating = mode !== "full";

  const player = (
    <div
      onMouseMove={nudgeControls}
      onMouseLeave={() => !videoRef.current?.paused && setControlsShown(false)}
      style={floating && effPos ? { left: effPos.x, top: effPos.y, width: WIDTHS[mode] } : undefined}
      className={cn(
        "group fixed z-[80] overflow-hidden bg-black shadow-2xl",
        floating ? "aspect-video rounded-xl border border-border" : "inset-0",
      )}
    >
      {/* eslint-disable-next-line jsx-a11y/media-has-caption */}
      <video
        ref={videoRef}
        playsInline
        onClick={togglePlay}
        className="absolute inset-0 size-full bg-black object-contain"
      />

      {/* Top bar — also the drag handle in floating modes. */}
      <div
        onPointerDown={onDragDown}
        onPointerMove={onDragMove}
        onPointerUp={onDragUp}
        className={cn(
          "absolute inset-x-0 top-0 flex items-center gap-2 bg-gradient-to-b from-black/70 to-transparent px-3 py-2 transition-opacity",
          floating && "cursor-move",
          controlsShown ? "opacity-100" : "opacity-0",
        )}
      >
        {channel.logo && (
          <img src={channel.logo} alt="" className="size-7 shrink-0 rounded bg-white/10 object-contain" />
        )}
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold text-white">{channel.name}</p>
          {mode !== "mini" && (
            <p className="truncate text-xs text-white/60">
              {[channel.group, channel.country, channel.quality].filter(Boolean).join(" · ")}
            </p>
          )}
        </div>
        <span className="flex items-center gap-1 rounded-full bg-red-600/90 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-white">
          <span className="size-1.5 rounded-full bg-white" />
          Live
        </span>
        <IconBtn label="Close player" onClick={onClose}>
          <X className="size-4" aria-hidden="true" />
        </IconBtn>
      </div>

      {/* Center state. */}
      <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
        {status === "error" ? (
          <div className="pointer-events-auto flex flex-col items-center gap-3 rounded-card bg-black/60 px-6 py-5 text-center">
            <AlertTriangle className="size-7 text-warning" aria-hidden="true" />
            <p className="max-w-xs text-sm text-white/80">
              This channel wouldn’t load. It may be offline or geo-restricted — try another.
            </p>
            <button
              type="button"
              onClick={() => setReloadKey((k) => k + 1)}
              className="flex items-center gap-2 rounded-control bg-white/10 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-white/20"
            >
              <RotateCw className="size-4" aria-hidden="true" />
              Try again
            </button>
          </div>
        ) : busy ? (
          <Loader2 className="size-9 animate-spin text-white/80" aria-hidden="true" />
        ) : status === "paused" ? (
          <button
            type="button"
            onClick={togglePlay}
            aria-label="Play"
            className="pointer-events-auto flex size-14 items-center justify-center rounded-full bg-black/50 text-white transition-transform hover:scale-105"
          >
            <Play className="size-7 translate-x-0.5" aria-hidden="true" />
          </button>
        ) : null}
      </div>

      {/* Bottom control bar. */}
      <div
        className={cn(
          "absolute inset-x-0 bottom-0 flex items-center gap-2 bg-gradient-to-t from-black/70 to-transparent px-3 py-2 transition-opacity",
          controlsShown ? "opacity-100" : "opacity-0",
        )}
      >
        <IconBtn label={status === "playing" ? "Pause" : "Play"} onClick={togglePlay}>
          {status === "playing" ? (
            <Pause className="size-5" aria-hidden="true" />
          ) : (
            <Play className="size-5 translate-x-0.5" aria-hidden="true" />
          )}
        </IconBtn>

        <IconBtn label={muted ? "Unmute" : "Mute"} onClick={toggleMute}>
          {muted || volume === 0 ? (
            <VolumeX className="size-5" aria-hidden="true" />
          ) : (
            <Volume2 className="size-5" aria-hidden="true" />
          )}
        </IconBtn>
        {mode !== "mini" && (
          <input
            type="range"
            min={0}
            max={1}
            step={0.02}
            value={muted ? 0 : volume}
            onChange={(e) => changeVolume(Number(e.target.value))}
            aria-label="Volume"
            className="h-1 w-20 cursor-pointer appearance-none rounded-full bg-white/30 accent-white"
          />
        )}

        <div className="flex-1" />

        {floating && (
          <IconBtn
            label={mode === "mini" ? "Larger" : "Mini player"}
            onClick={() => setMode(mode === "mini" ? "normal" : "mini")}
          >
            {mode === "mini" ? (
              <Maximize2 className="size-4" aria-hidden="true" />
            ) : (
              <Minimize2 className="size-4" aria-hidden="true" />
            )}
          </IconBtn>
        )}
        <IconBtn
          label={mode === "full" ? "Exit fullscreen" : "Fullscreen"}
          onClick={() => setMode(mode === "full" ? "normal" : "full")}
        >
          {mode === "full" ? (
            <Minimize className="size-5" aria-hidden="true" />
          ) : (
            <Maximize className="size-5" aria-hidden="true" />
          )}
        </IconBtn>
      </div>
    </div>
  );

  return createPortal(player, document.body);
}

/** A round translucent control button. */
function IconBtn({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      // Don't let a click on a control start a drag on the bar behind it.
      onPointerDown={(e) => e.stopPropagation()}
      onClick={onClick}
      className="flex size-8 shrink-0 items-center justify-center rounded-full text-white/90 transition-colors hover:bg-white/15 hover:text-white"
    >
      {children}
    </button>
  );
}

function heightFor(mode: Exclude<Mode, "full">): number {
  return Math.round((WIDTHS[mode] * 9) / 16);
}

function defaultPos(mode: Exclude<Mode, "full">): { x: number; y: number } {
  return {
    x: window.innerWidth - WIDTHS[mode] - MARGIN,
    y: window.innerHeight - heightFor(mode) - MARGIN,
  };
}

function clamp(p: { x: number; y: number }, mode: Mode): { x: number; y: number } {
  if (mode === "full") return p;
  const w = WIDTHS[mode];
  const h = heightFor(mode);
  return {
    x: Math.max(0, Math.min(p.x, window.innerWidth - w)),
    y: Math.max(0, Math.min(p.y, window.innerHeight - h)),
  };
}
