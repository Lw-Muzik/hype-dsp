import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AlertTriangle,
  Loader2,
  Maximize,
  Minimize,
  Pause,
  Play as PlayIcon,
  RotateCw,
  SkipBack,
  SkipForward,
  Volume1,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { formatTime } from "@/lib/format";
import { cn } from "@/lib/cn";
import type { MovieStream } from "@/lib/types";
import { availableQualities, selectStream, type Quality } from "./streamSelect";

type Status = "loading" | "playing" | "paused" | "error";

/**
 * The in-app movie player.
 *
 * Unlike `VideoStage` (the muted, engine-synced picture for YT Music videos),
 * this `<video>` carries its own real audio — an Archive movie file is a
 * complete progressive MP4, Range-supported straight off `archive.org/download`,
 * with no DSP chain in front of it. So it owns the audio route itself rather
 * than following the engine's clock: the moment it mounts it stops the engine,
 * the same way `TvPlayer` stops it before live TV takes over the speakers.
 *
 * Fullscreen borrows `VideoStage`'s reasoning instead of `TvPlayer`'s portal:
 * it's a CSS state (`fixed inset-0`) on the very same element, never a portal
 * or a DOM move, so toggling it never remounts the `<video>` and interrupts
 * what's already buffered and playing.
 */
export function MoviePlayer({
  title,
  poster,
  streams,
  initialQuality,
  onClose,
}: {
  title: string;
  poster: string;
  streams: MovieStream[];
  initialQuality: Quality;
  onClose: () => void;
}) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const stopEngine = useEngineStore((s) => s.stop);

  const [quality, setQuality] = useState<Quality>(initialQuality);
  const [status, setStatus] = useState<Status>("loading");
  const [muted, setMuted] = useState(false);
  const [volume, setVolume] = useState(1);
  const [fullscreen, setFullscreen] = useState(false);
  const [duration, setDuration] = useState(0);
  const [currentTime, setCurrentTime] = useState(0);
  const [buffered, setBuffered] = useState(0);
  const [reloadKey, setReloadKey] = useState(0);

  // The live fullscreen value for the unmount cleanup, which would otherwise
  // close over the initial `false` and leave the OS window stuck fullscreen.
  const fullscreenRef = useRef(false);
  fullscreenRef.current = fullscreen;

  const qualities = availableQualities(streams);
  const stream = selectStream(streams, quality);

  // Own the audio route the instant a movie starts: a movie has real audio of
  // its own, and it must never fight the DSP engine for the output device.
  // Deliberately once per mount — `MovieDetailView` remounts a fresh player
  // (via `key`) per movie, so there's no later movie to react to here.
  useEffect(() => {
    void stopEngine();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Load the chosen stream. Position and play state carry across a quality
  // switch (SD <-> HD), so toggling mid-watch resumes instead of restarting.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    if (!stream) {
      setStatus("error");
      return;
    }
    let cancelled = false;
    const resumeAt = video.currentTime > 0 ? video.currentTime : null;
    const wasPlaying = !video.paused && resumeAt !== null;
    setStatus("loading");

    const onLoaded = () => {
      if (cancelled) return;
      if (resumeAt !== null) {
        video.currentTime = Math.min(resumeAt, isFinite(video.duration) ? video.duration : resumeAt);
      }
      if (wasPlaying || resumeAt === null) {
        video.play().catch(() => {
          video.muted = true;
          setMuted(true);
          void video.play().catch(() => {});
        });
      }
    };
    const onLoadError = () => {
      if (!cancelled) setStatus("error");
    };
    video.addEventListener("loadedmetadata", onLoaded, { once: true });
    video.addEventListener("error", onLoadError, { once: true });
    video.src = stream.url;
    video.load();

    return () => {
      cancelled = true;
      video.removeEventListener("loadedmetadata", onLoaded);
      video.removeEventListener("error", onLoadError);
    };
  }, [stream?.url, reloadKey]);

  // Reflect the element's own state.
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
    const onTime = () => setCurrentTime(video.currentTime);
    const onDuration = () => setDuration(video.duration);
    const onProgress = () => {
      const b = video.buffered;
      setBuffered(b.length ? b.end(b.length - 1) : 0);
    };
    video.addEventListener("playing", onPlaying);
    video.addEventListener("waiting", onWaiting);
    video.addEventListener("pause", onPause);
    video.addEventListener("error", onError);
    video.addEventListener("volumechange", onVolume);
    video.addEventListener("timeupdate", onTime);
    video.addEventListener("durationchange", onDuration);
    video.addEventListener("progress", onProgress);
    return () => {
      video.removeEventListener("playing", onPlaying);
      video.removeEventListener("waiting", onWaiting);
      video.removeEventListener("pause", onPause);
      video.removeEventListener("error", onError);
      video.removeEventListener("volumechange", onVolume);
      video.removeEventListener("timeupdate", onTime);
      video.removeEventListener("durationchange", onDuration);
      video.removeEventListener("progress", onProgress);
    };
  }, []);

  const toggleFullscreen = useCallback(() => {
    setFullscreen((on) => {
      const next = !on;
      void getCurrentWindow().setFullscreen(next).catch(() => {});
      return next;
    });
  }, []);

  // Leaving the player must not strand the OS window in fullscreen.
  useEffect(() => {
    return () => {
      if (fullscreenRef.current) void getCurrentWindow().setFullscreen(false).catch(() => {});
    };
  }, []);

  const togglePlay = useCallback(() => {
    const v = videoRef.current;
    if (!v) return;
    if (v.paused) void v.play().catch(() => {});
    else v.pause();
  }, []);
  const toggleMute = useCallback(() => {
    const v = videoRef.current;
    if (v) v.muted = !v.muted;
  }, []);
  const changeVolume = (value: number) => {
    const v = videoRef.current;
    if (!v) return;
    v.volume = value;
    v.muted = value === 0;
  };
  const seekTo = (t: number) => {
    const v = videoRef.current;
    if (v) v.currentTime = t;
  };
  const skip = useCallback((delta: number) => {
    const v = videoRef.current;
    if (v && isFinite(v.duration)) {
      v.currentTime = Math.max(0, Math.min(v.duration, v.currentTime + delta));
    }
  }, []);

  // Keyboard shortcuts, ignored while typing in a field — the same set
  // `TvPlayer` offers, minus what a VOD file has no use for.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = document.activeElement;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) return;
      switch (e.key) {
        case "Escape":
          if (fullscreen) toggleFullscreen();
          break;
        case " ":
        case "k":
          e.preventDefault();
          togglePlay();
          break;
        case "m":
          toggleMute();
          break;
        case "f":
          toggleFullscreen();
          break;
        case "ArrowLeft":
          skip(-10);
          break;
        case "ArrowRight":
          skip(10);
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fullscreen, toggleFullscreen, togglePlay, toggleMute, skip]);

  const VolIcon = muted || volume === 0 ? VolumeX : volume < 0.5 ? Volume1 : Volume2;
  const busy = status === "loading";

  return (
    <div
      className={cn(
        "group/player relative w-full overflow-hidden bg-black",
        fullscreen ? "fixed inset-0 z-[100]" : "aspect-video rounded-card ring-1 ring-border",
      )}
    >
      {/* eslint-disable-next-line jsx-a11y/media-has-caption */}
      <video
        ref={videoRef}
        poster={poster}
        playsInline
        controls={false}
        onClick={togglePlay}
        className="size-full bg-black object-contain"
        aria-label={title}
      />

      <div className="absolute inset-x-0 top-0 flex items-center gap-2 bg-gradient-to-b from-black/70 to-transparent px-3 py-2 opacity-0 transition-opacity group-hover/player:opacity-100">
        <p className="min-w-0 flex-1 truncate text-sm font-semibold text-white">{title}</p>
        <button type="button" aria-label="Close player" onClick={onClose} className={overlayBtn}>
          <X className="size-4" aria-hidden="true" />
        </button>
      </div>

      <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
        {status === "error" ? (
          <div className="pointer-events-auto flex flex-col items-center gap-3 rounded-card bg-black/60 px-6 py-5 text-center">
            <AlertTriangle className="size-7 text-warning" aria-hidden="true" />
            <p className="max-w-xs text-sm text-white/80">
              {stream
                ? "This film wouldn’t load."
                : "No playable version of this film is available."}
            </p>
            {stream && (
              <button
                type="button"
                onClick={() => setReloadKey((k) => k + 1)}
                className="flex items-center gap-2 rounded-control bg-white/10 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-white/20"
              >
                <RotateCw className="size-4" aria-hidden="true" />
                Try again
              </button>
            )}
          </div>
        ) : busy ? (
          <Loader2 className="size-9 animate-spin text-white/80" aria-hidden="true" />
        ) : status === "paused" ? (
          <button
            type="button"
            onClick={togglePlay}
            aria-label="Play"
            className="pointer-events-auto flex size-16 items-center justify-center rounded-full bg-black/50 text-white transition-transform hover:scale-105"
          >
            <PlayIcon className="size-8 translate-x-0.5" aria-hidden="true" />
          </button>
        ) : null}
      </div>

      <div className="absolute inset-x-0 bottom-0 flex flex-col gap-1.5 bg-gradient-to-t from-black/80 to-transparent px-3 pb-2 pt-8 opacity-0 transition-opacity focus-within:opacity-100 group-hover/player:opacity-100">
        <div className="flex items-center gap-2 text-[11px] tabular-nums text-white/80">
          <span>{formatTime(currentTime)}</span>
          <div className="relative flex-1">
            <div className="absolute inset-y-1/2 h-1 w-full -translate-y-1/2 rounded-full bg-white/25" />
            <div
              className="absolute inset-y-1/2 h-1 -translate-y-1/2 rounded-full bg-white/40"
              style={{ width: `${duration ? (buffered / duration) * 100 : 0}%` }}
            />
            <input
              type="range"
              min={0}
              max={duration || 0}
              step={0.1}
              value={currentTime}
              onChange={(e) => seekTo(Number(e.target.value))}
              aria-label="Seek"
              className="relative h-1 w-full cursor-pointer appearance-none rounded-full bg-transparent accent-accent"
            />
          </div>
          <span>{formatTime(duration)}</span>
        </div>

        <div className="flex items-center gap-1.5">
          <button
            type="button"
            aria-label={status === "playing" ? "Pause" : "Play"}
            onClick={togglePlay}
            className={overlayBtn}
          >
            {status === "playing" ? (
              <Pause className="size-5" aria-hidden="true" />
            ) : (
              <PlayIcon className="size-5 translate-x-0.5" aria-hidden="true" />
            )}
          </button>
          <button type="button" aria-label="Back 10 seconds" onClick={() => skip(-10)} className={overlayBtn}>
            <SkipBack className="size-4" aria-hidden="true" />
          </button>
          <button type="button" aria-label="Forward 10 seconds" onClick={() => skip(10)} className={overlayBtn}>
            <SkipForward className="size-4" aria-hidden="true" />
          </button>
          <button type="button" aria-label={muted ? "Unmute" : "Mute"} onClick={toggleMute} className={overlayBtn}>
            <VolIcon className="size-5" aria-hidden="true" />
          </button>
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

          <div className="flex-1" />

          {qualities.length > 1 && (
            <div role="radiogroup" aria-label="Video quality" className="flex items-center gap-1 rounded-full bg-white/10 p-0.5">
              {qualities.map((q) => (
                <button
                  key={q}
                  type="button"
                  role="radio"
                  aria-checked={quality === q}
                  onClick={() => setQuality(q)}
                  className={cn(
                    "rounded-full px-2.5 py-1 text-xs font-medium transition-colors",
                    quality === q ? "bg-white/25 text-white" : "text-white/70 hover:text-white",
                  )}
                >
                  {q}
                </button>
              ))}
            </div>
          )}

          <button
            type="button"
            aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
            onClick={toggleFullscreen}
            className={overlayBtn}
          >
            {fullscreen ? <Minimize className="size-4" aria-hidden="true" /> : <Maximize className="size-4" aria-hidden="true" />}
          </button>
        </div>
      </div>
    </div>
  );
}

const overlayBtn =
  "flex size-8 shrink-0 items-center justify-center rounded-full text-white/80 transition-colors hover:bg-white/15 hover:text-white disabled:pointer-events-none disabled:opacity-30";
