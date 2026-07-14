import { useCallback, useEffect, useRef, useState } from "react";
import type HlsType from "hls.js";
import {
  AlertTriangle,
  Loader2,
  Maximize,
  Minimize,
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

/**
 * In-app TV player. Plays a channel in an embedded `<video>` via hls.js (with a
 * native-HLS fallback for WKWebView), fed by the local HLS proxy — no native
 * window, no external player. Starting a channel stops the app's audio engine so
 * the two don't overlap.
 */
export function TvPlayer({ channel, onClose }: { channel: TvChannel; onClose: () => void }) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const hlsRef = useRef<HlsType | null>(null);
  const hideTimer = useRef<number | null>(null);

  const [status, setStatus] = useState<Status>("loading");
  const [muted, setMuted] = useState(false);
  const [volume, setVolume] = useState(1);
  const [expanded, setExpanded] = useState(false);
  const [controlsShown, setControlsShown] = useState(true);
  const [reloadKey, setReloadKey] = useState(0);

  const stopEngine = useEngineStore((s) => s.stop);

  // Load / switch the stream whenever the channel (or a manual retry) changes.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    let cancelled = false;
    setStatus("loading");
    void stopEngine(); // don't stack TV audio on top of the app engine

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
      // Try with sound; if the webview blocks unmuted autoplay, fall back to
      // muted playback (the user can unmute with a gesture).
      video.play().catch(() => {
        video.muted = true;
        setMuted(true);
        void video.play().catch(() => {});
      });
    };

    tvStreamUrl(channel)
      .then(async (url) => {
        if (cancelled) return;
        // Lazy-load hls.js only when a channel actually plays (keeps the startup
        // bundle lean). Fall back to native HLS on WKWebView / Safari.
        const { default: Hls } = await import("hls.js");
        if (cancelled) return;
        if (Hls.isSupported()) {
          const hls = new Hls({ lowLatencyMode: true, enableWorker: true, backBufferLength: 30 });
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
          video.src = url; // native HLS (WKWebView / Safari)
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

  // Escape exits the expanded (in-app fullscreen) view.
  useEffect(() => {
    if (!expanded) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setExpanded(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [expanded]);

  // Auto-hide the controls a couple seconds after the pointer goes idle while
  // playing; any movement brings them back.
  const nudgeControls = useCallback(() => {
    setControlsShown(true);
    if (hideTimer.current) window.clearTimeout(hideTimer.current);
    hideTimer.current = window.setTimeout(() => {
      if (!videoRef.current?.paused) setControlsShown(false);
    }, 2600);
  }, []);

  useEffect(() => () => {
    if (hideTimer.current) window.clearTimeout(hideTimer.current);
  }, []);

  const togglePlay = () => {
    const v = videoRef.current;
    if (!v) return;
    if (v.paused) void v.play().catch(() => {});
    else v.pause();
  };

  const toggleMute = () => {
    const v = videoRef.current;
    if (!v) return;
    v.muted = !v.muted;
  };

  const changeVolume = (value: number) => {
    const v = videoRef.current;
    if (!v) return;
    v.volume = value;
    v.muted = value === 0;
  };

  const toggleExpanded = () => setExpanded((e) => !e);

  const busy = status === "loading";

  return (
    <div
      ref={containerRef}
      onMouseMove={nudgeControls}
      onMouseLeave={() => !videoRef.current?.paused && setControlsShown(false)}
      className={cn(
        "group relative overflow-hidden border border-border bg-black",
        expanded
          ? "fixed inset-0 z-50 aspect-auto rounded-none border-0"
          : "aspect-video w-full rounded-card",
      )}
    >
      {/* eslint-disable-next-line jsx-a11y/media-has-caption */}
      <video
        ref={videoRef}
        playsInline
        onClick={togglePlay}
        className="absolute inset-0 size-full bg-black object-contain"
      />

      {/* Top overlay: channel identity + close. */}
      <div
        className={cn(
          "pointer-events-none absolute inset-x-0 top-0 flex items-center gap-3 bg-gradient-to-b from-black/70 to-transparent px-4 py-3 transition-opacity",
          controlsShown ? "opacity-100" : "opacity-0",
        )}
      >
        {channel.logo && (
          <img
            src={channel.logo}
            alt=""
            className="size-8 shrink-0 rounded bg-white/10 object-contain"
          />
        )}
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold text-white">{channel.name}</p>
          <p className="truncate text-xs text-white/60">
            {[channel.group, channel.country, channel.quality].filter(Boolean).join(" · ")}
          </p>
        </div>
        <span className="flex items-center gap-1.5 rounded-full bg-red-600/90 px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wide text-white">
          <span className="size-1.5 rounded-full bg-white" />
          Live
        </span>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close player"
          className="pointer-events-auto flex size-8 items-center justify-center rounded-full bg-black/40 text-white/80 transition-colors hover:bg-black/60 hover:text-white"
        >
          <X className="size-4" aria-hidden="true" />
        </button>
      </div>

      {/* Center state: spinner / play / error. */}
      <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
        {status === "error" ? (
          <div className="pointer-events-auto flex flex-col items-center gap-3 rounded-card bg-black/60 px-6 py-5 text-center">
            <AlertTriangle className="size-8 text-warning" aria-hidden="true" />
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
          <Loader2 className="size-10 animate-spin text-white/80" aria-hidden="true" />
        ) : status === "paused" ? (
          <button
            type="button"
            onClick={togglePlay}
            aria-label="Play"
            className="pointer-events-auto flex size-16 items-center justify-center rounded-full bg-black/50 text-white transition-transform hover:scale-105"
          >
            <Play className="size-8 translate-x-0.5" aria-hidden="true" />
          </button>
        ) : null}
      </div>

      {/* Bottom control bar. */}
      <div
        className={cn(
          "absolute inset-x-0 bottom-0 flex items-center gap-3 bg-gradient-to-t from-black/70 to-transparent px-4 py-3 transition-opacity",
          controlsShown ? "opacity-100" : "opacity-0",
        )}
      >
        <button
          type="button"
          onClick={togglePlay}
          aria-label={status === "playing" ? "Pause" : "Play"}
          className="flex size-9 items-center justify-center rounded-full text-white transition-colors hover:bg-white/15"
        >
          {status === "playing" ? (
            <Pause className="size-5" aria-hidden="true" />
          ) : (
            <Play className="size-5 translate-x-0.5" aria-hidden="true" />
          )}
        </button>

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={toggleMute}
            aria-label={muted ? "Unmute" : "Mute"}
            className="flex size-9 items-center justify-center rounded-full text-white transition-colors hover:bg-white/15"
          >
            {muted || volume === 0 ? (
              <VolumeX className="size-5" aria-hidden="true" />
            ) : (
              <Volume2 className="size-5" aria-hidden="true" />
            )}
          </button>
          <input
            type="range"
            min={0}
            max={1}
            step={0.02}
            value={muted ? 0 : volume}
            onChange={(e) => changeVolume(Number(e.target.value))}
            aria-label="Volume"
            className="h-1 w-24 cursor-pointer appearance-none rounded-full bg-white/30 accent-white"
          />
        </div>

        <div className="flex-1" />

        <button
          type="button"
          onClick={toggleExpanded}
          aria-label={expanded ? "Exit fullscreen" : "Fullscreen"}
          className="flex size-9 items-center justify-center rounded-full text-white transition-colors hover:bg-white/15"
        >
          {expanded ? (
            <Minimize className="size-5" aria-hidden="true" />
          ) : (
            <Maximize className="size-5" aria-hidden="true" />
          )}
        </button>
      </div>
    </div>
  );
}
