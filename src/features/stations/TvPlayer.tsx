import { useCallback, useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent, ReactNode } from "react";
import { createPortal } from "react-dom";
import type HlsType from "hls.js";
import {
  AlertTriangle,
  Check,
  Loader2,
  Maximize,
  Maximize2,
  Minimize,
  Minimize2,
  Pause,
  PictureInPicture2,
  Play,
  RotateCw,
  Settings,
  SkipBack,
  SkipForward,
  Volume1,
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
type Level = { index: number; label: string };
type Track = { id: number; label: string };

const WIDTHS: Record<Exclude<Mode, "full">, number> = { normal: 448, mini: 288 };
const MARGIN = 24;

function fmtTime(s: number): string {
  if (!isFinite(s) || s < 0) return "0:00";
  const total = Math.floor(s);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = total % 60;
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  return `${h > 0 ? `${h}:` : ""}${mm}:${String(sec).padStart(2, "0")}`;
}

/**
 * In-app TV player: an embedded `<video>` (hls.js, native-HLS fallback) fed by
 * the local HLS proxy — no native window. Renders through a portal to
 * document.body as a draggable picture-in-picture (mini / normal / fullscreen)
 * so the channel list stays browsable; selecting another channel re-points the
 * same element. Full control set: play/pause, seek + time (VOD/DVR), volume,
 * quality, audio tracks, PiP, keyboard shortcuts, fullscreen.
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

  const [isLive, setIsLive] = useState(true);
  const [duration, setDuration] = useState(0);
  const [currentTime, setCurrentTime] = useState(0);
  const [buffered, setBuffered] = useState(0);

  const [levels, setLevels] = useState<Level[]>([]);
  const [currentLevel, setCurrentLevel] = useState(-1); // -1 = auto
  const [audioTracks, setAudioTracks] = useState<Track[]>([]);
  const [audioTrack, setAudioTrack] = useState(-1);
  const [subtitleTracks, setSubtitleTracks] = useState<Track[]>([]);
  const [subtitleTrack, setSubtitleTrack] = useState(-1); // -1 = off
  const [speed, setSpeed] = useState(1);

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pipActive, setPipActive] = useState(false);

  const stopEngine = useEngineStore((s) => s.stop);

  // Load / switch the stream whenever the channel (or a manual retry) changes.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    let cancelled = false;
    // How many times we'll try to recover from a fatal error before calling the
    // channel dead. hls.js's default reaction to a fatal network error is to
    // reload forever — for a stream that is genuinely down (and iptv-org ships
    // many), that is an eternal spinner the user reads as "the app is broken"
    // rather than "this channel is". A small budget lets a real transient blip
    // recover while a dead channel fails fast and honestly.
    const MAX_RECOVERIES = 3;
    let recoveries = 0;
    // Nothing has played within this long ⇒ dead, even if hls.js never emitted a
    // fatal error. This is the safety net for the case the backend probe can't
    // catch: a master playlist that loads but whose segments 403/stall, which
    // otherwise buffers silently forever.
    const STALL_DEADLINE_MS = 15_000;
    let stallTimer: number | undefined;
    const armStall = () => {
      window.clearTimeout(stallTimer);
      stallTimer = window.setTimeout(() => {
        if (!cancelled) setStatus("error");
      }, STALL_DEADLINE_MS);
    };
    setStatus("loading");
    setLevels([]);
    setAudioTracks([]);
    setSubtitleTracks([]);
    setSubtitleTrack(-1);
    setSpeed(1);
    setIsLive(true);
    void stopEngine();

    const teardown = () => {
      window.clearTimeout(stallTimer);
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

    // Real playback disarms the stall deadline and refills the recovery budget,
    // so a stream that plays, blips, and recovers isn't punished for the blip.
    const onPlaying = () => {
      window.clearTimeout(stallTimer);
      recoveries = 0;
    };
    video.addEventListener("playing", onPlaying);

    tvStreamUrl(channel)
      .then(async (url) => {
        if (cancelled) return;
        const { default: Hls } = await import("hls.js");
        if (cancelled) return;
        if (Hls.isSupported()) {
          const hls = new Hls({
            enableWorker: true,
            startFragPrefetch: true,
            lowLatencyMode: false,
            backBufferLength: 30,
            manifestLoadingTimeOut: 8000,
            manifestLoadingMaxRetry: 3,
            levelLoadingTimeOut: 8000,
            fragLoadingTimeOut: 20000,
          });
          hlsRef.current = hls;
          hls.loadSource(url);
          hls.attachMedia(video);
          armStall();
          hls.on(Hls.Events.MANIFEST_PARSED, () => {
            play();
            setLevels(
              hls.levels.map((l, i) => ({
                index: i,
                label: l.height ? `${l.height}p` : `${Math.round((l.bitrate || 0) / 1000)}k`,
              })),
            );
          });
          hls.on(Hls.Events.LEVEL_SWITCHED, (_e, data) => {
            setCurrentLevel(hls.autoLevelEnabled ? -1 : data.level);
          });
          hls.on(Hls.Events.LEVEL_LOADED, (_e, data) => setIsLive(data.details.live));
          hls.on(Hls.Events.AUDIO_TRACKS_UPDATED, () => {
            setAudioTracks(
              hls.audioTracks.map((t) => ({ id: t.id, label: t.name || t.lang || `Track ${t.id + 1}` })),
            );
            setAudioTrack(hls.audioTrack);
          });
          hls.on(Hls.Events.AUDIO_TRACK_SWITCHED, (_e, data) => setAudioTrack(data.id));
          hls.on(Hls.Events.SUBTITLE_TRACKS_UPDATED, () => {
            setSubtitleTracks(
              hls.subtitleTracks.map((t) => ({ id: t.id, label: t.name || t.lang || `Track ${t.id + 1}` })),
            );
          });
          hls.on(Hls.Events.SUBTITLE_TRACK_SWITCH, (_e, data) => setSubtitleTrack(data.id));
          hls.on(Hls.Events.ERROR, (_e, data) => {
            if (!data.fatal) return;
            // Give recovery a bounded number of tries, then stop. Without the
            // budget, a fatal network error on a dead stream loops here forever.
            if (recoveries >= MAX_RECOVERIES) {
              window.clearTimeout(stallTimer);
              setStatus("error");
              return;
            }
            recoveries += 1;
            if (data.type === Hls.ErrorTypes.NETWORK_ERROR) {
              armStall();
              hls.startLoad();
            } else if (data.type === Hls.ErrorTypes.MEDIA_ERROR) {
              hls.recoverMediaError();
            } else {
              window.clearTimeout(stallTimer);
              setStatus("error");
            }
          });
        } else if (video.canPlayType("application/vnd.apple.mpegurl")) {
          video.src = url;
          armStall();
          video.addEventListener("loadedmetadata", play, { once: true });
          // Native HLS surfaces a load failure as a media `error` event; without
          // this the WKWebView fallback path would spin forever too.
          video.addEventListener(
            "error",
            () => {
              if (!cancelled) setStatus("error");
            },
            { once: true },
          );
        } else {
          setStatus("error");
        }
      })
      .catch(() => {
        if (!cancelled) setStatus("error");
      });

    return () => {
      cancelled = true;
      video.removeEventListener("playing", onPlaying);
      teardown();
    };
  }, [channel, reloadKey, stopEngine]);

  // Reflect the media element's state.
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
    const onDuration = () => {
      setDuration(video.duration);
      if (!isFinite(video.duration)) setIsLive(true);
    };
    const onProgress = () => {
      const b = video.buffered;
      setBuffered(b.length ? b.end(b.length - 1) : 0);
    };
    const onEnterPip = () => setPipActive(true);
    const onLeavePip = () => setPipActive(false);
    video.addEventListener("playing", onPlaying);
    video.addEventListener("waiting", onWaiting);
    video.addEventListener("pause", onPause);
    video.addEventListener("error", onError);
    video.addEventListener("volumechange", onVolume);
    video.addEventListener("timeupdate", onTime);
    video.addEventListener("durationchange", onDuration);
    video.addEventListener("progress", onProgress);
    video.addEventListener("enterpictureinpicture", onEnterPip);
    video.addEventListener("leavepictureinpicture", onLeavePip);
    return () => {
      video.removeEventListener("playing", onPlaying);
      video.removeEventListener("waiting", onWaiting);
      video.removeEventListener("pause", onPause);
      video.removeEventListener("error", onError);
      video.removeEventListener("volumechange", onVolume);
      video.removeEventListener("timeupdate", onTime);
      video.removeEventListener("durationchange", onDuration);
      video.removeEventListener("progress", onProgress);
      video.removeEventListener("enterpictureinpicture", onEnterPip);
      video.removeEventListener("leavepictureinpicture", onLeavePip);
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
  const nudgeVolume = useCallback((delta: number) => {
    const v = videoRef.current;
    if (!v) return;
    v.volume = Math.max(0, Math.min(1, v.volume + delta));
    v.muted = false;
  }, []);
  const seekTo = (t: number) => {
    const v = videoRef.current;
    if (v) v.currentTime = t;
  };
  const skip = useCallback((delta: number) => {
    const v = videoRef.current;
    if (v && isFinite(v.duration)) v.currentTime = Math.max(0, Math.min(v.duration, v.currentTime + delta));
  }, []);

  const setLevel = (index: number) => {
    if (hlsRef.current) hlsRef.current.currentLevel = index; // -1 = auto
    setCurrentLevel(index);
  };
  const setAudio = (id: number) => {
    if (hlsRef.current) hlsRef.current.audioTrack = id;
    setAudioTrack(id);
  };
  const setSubtitle = (id: number) => {
    const hls = hlsRef.current;
    if (hls) {
      hls.subtitleTrack = id;
      hls.subtitleDisplay = id !== -1;
    }
    setSubtitleTrack(id);
  };
  const changeSpeed = (rate: number) => {
    const v = videoRef.current;
    if (v) v.playbackRate = rate;
    setSpeed(rate);
  };
  const togglePip = async () => {
    const v = videoRef.current;
    if (!v) return;
    try {
      if (document.pictureInPictureElement) await document.exitPictureInPicture();
      else await v.requestPictureInPicture();
    } catch {
      /* PiP unsupported/blocked — ignore. */
    }
  };

  // --- Fullscreen escape + drag + resize clamp + auto-hide -----------------
  useEffect(() => {
    if (mode !== "full") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMode("normal");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [mode]);

  // Global keyboard shortcuts (ignored while typing in a field).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = document.activeElement;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || (el as HTMLElement).isContentEditable))
        return;
      switch (e.key) {
        case " ":
        case "k":
          e.preventDefault();
          togglePlay();
          break;
        case "m":
          toggleMute();
          break;
        case "f":
          setMode((m) => (m === "full" ? "normal" : "full"));
          break;
        case "ArrowUp":
          e.preventDefault();
          nudgeVolume(0.05);
          break;
        case "ArrowDown":
          e.preventDefault();
          nudgeVolume(-0.05);
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
  }, [togglePlay, toggleMute, nudgeVolume, skip]);

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
      if (!videoRef.current?.paused && !settingsOpen) setControlsShown(false);
    }, 2600);
  }, [settingsOpen]);

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
  const compact = mode === "mini";
  const seekable = !isLive && isFinite(duration) && duration > 1;
  const pipSupported = typeof document !== "undefined" && document.pictureInPictureEnabled;
  const hasSettings = levels.length > 1 || audioTracks.length > 1 || subtitleTracks.length > 0 || seekable;
  const VolIcon = muted || volume === 0 ? VolumeX : volume < 0.5 ? Volume1 : Volume2;

  const player = (
    <div
      onMouseMove={nudgeControls}
      onMouseLeave={() => !videoRef.current?.paused && !settingsOpen && setControlsShown(false)}
      style={floating && effPos ? { left: effPos.x, top: effPos.y, width: WIDTHS[mode] } : undefined}
      className={cn(
        "group fixed z-[80] overflow-hidden bg-black shadow-2xl",
        floating ? "aspect-video rounded-xl border border-border" : "inset-0",
      )}
    >
      {/* eslint-disable-next-line jsx-a11y/media-has-caption */}
      <video ref={videoRef} playsInline onClick={togglePlay} className="absolute inset-0 size-full bg-black object-contain" />

      {/* Top bar (drag handle in floating modes). */}
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
        {channel.logo && <img src={channel.logo} alt="" className="size-7 shrink-0 rounded bg-white/10 object-contain" />}
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold text-white">{channel.name}</p>
          {!compact && (
            <p className="truncate text-xs text-white/60">
              {[channel.group, channel.country, channel.quality].filter(Boolean).join(" · ")}
            </p>
          )}
        </div>
        {isLive && (
          <span className="flex items-center gap-1 rounded-full bg-red-600/90 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-white">
            <span className="size-1.5 rounded-full bg-white" />
            Live
          </span>
        )}
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

      {/* Bottom controls. */}
      <div
        className={cn(
          "absolute inset-x-0 bottom-0 flex flex-col gap-1.5 bg-gradient-to-t from-black/80 to-transparent px-3 pb-2 pt-6 transition-opacity",
          controlsShown ? "opacity-100" : "opacity-0",
        )}
      >
        {/* Seek bar (VOD / DVR only). */}
        {seekable && !compact && (
          <div className="flex items-center gap-2 text-[11px] tabular-nums text-white/80">
            <span>{fmtTime(currentTime)}</span>
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
            <span>{fmtTime(duration)}</span>
          </div>
        )}

        <div className="flex items-center gap-1.5">
          <IconBtn label={status === "playing" ? "Pause" : "Play"} onClick={togglePlay}>
            {status === "playing" ? <Pause className="size-5" aria-hidden="true" /> : <Play className="size-5 translate-x-0.5" aria-hidden="true" />}
          </IconBtn>

          {seekable && !compact && (
            <>
              <IconBtn label="Back 10 seconds" onClick={() => skip(-10)}>
                <SkipBack className="size-4" aria-hidden="true" />
              </IconBtn>
              <IconBtn label="Forward 10 seconds" onClick={() => skip(10)}>
                <SkipForward className="size-4" aria-hidden="true" />
              </IconBtn>
            </>
          )}

          <IconBtn label={muted ? "Unmute" : "Mute"} onClick={toggleMute}>
            <VolIcon className="size-5" aria-hidden="true" />
          </IconBtn>
          {!compact && (
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

          {!compact && hasSettings && (
            <div className="relative">
              <IconBtn label="Settings" onClick={() => setSettingsOpen((o) => !o)}>
                <Settings className="size-5" aria-hidden="true" />
              </IconBtn>
              {settingsOpen && (
                <SettingsMenu
                  levels={levels}
                  currentLevel={currentLevel}
                  onLevel={(i) => {
                    setLevel(i);
                    setSettingsOpen(false);
                  }}
                  audioTracks={audioTracks}
                  audioTrack={audioTrack}
                  onAudio={(id) => {
                    setAudio(id);
                    setSettingsOpen(false);
                  }}
                  subtitleTracks={subtitleTracks}
                  subtitleTrack={subtitleTrack}
                  onSubtitle={(id) => {
                    setSubtitle(id);
                    setSettingsOpen(false);
                  }}
                  seekable={seekable}
                  speed={speed}
                  onSpeed={(r) => {
                    changeSpeed(r);
                    setSettingsOpen(false);
                  }}
                  onClose={() => setSettingsOpen(false)}
                />
              )}
            </div>
          )}

          {!compact && pipSupported && (
            <IconBtn label="Picture in picture" onClick={togglePip}>
              <PictureInPicture2 className={cn("size-5", pipActive && "text-accent-strong")} aria-hidden="true" />
            </IconBtn>
          )}

          {floating && (
            <IconBtn label={compact ? "Larger" : "Mini player"} onClick={() => setMode(compact ? "normal" : "mini")}>
              {compact ? <Maximize2 className="size-4" aria-hidden="true" /> : <Minimize2 className="size-4" aria-hidden="true" />}
            </IconBtn>
          )}
          <IconBtn label={mode === "full" ? "Exit fullscreen" : "Fullscreen"} onClick={() => setMode(mode === "full" ? "normal" : "full")}>
            {mode === "full" ? <Minimize className="size-5" aria-hidden="true" /> : <Maximize className="size-5" aria-hidden="true" />}
          </IconBtn>
        </div>
      </div>
    </div>
  );

  return createPortal(player, document.body);
}

const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];

function SettingsMenu({
  levels,
  currentLevel,
  onLevel,
  audioTracks,
  audioTrack,
  onAudio,
  subtitleTracks,
  subtitleTrack,
  onSubtitle,
  seekable,
  speed,
  onSpeed,
  onClose,
}: {
  levels: Level[];
  currentLevel: number;
  onLevel: (index: number) => void;
  audioTracks: Track[];
  audioTrack: number;
  onAudio: (id: number) => void;
  subtitleTracks: Track[];
  subtitleTrack: number;
  onSubtitle: (id: number) => void;
  seekable: boolean;
  speed: number;
  onSpeed: (rate: number) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="absolute bottom-11 right-0 max-h-80 w-48 overflow-y-auto rounded-card border border-white/15 bg-neutral-900/95 py-1 text-sm text-white shadow-2xl backdrop-blur"
    >
      {levels.length > 1 && (
        <Section title="Quality">
          <MenuItem selected={currentLevel === -1} onClick={() => onLevel(-1)} label="Auto" />
          {levels.map((l) => (
            <MenuItem key={l.index} selected={currentLevel === l.index} onClick={() => onLevel(l.index)} label={l.label} />
          ))}
        </Section>
      )}
      {audioTracks.length > 1 && (
        <Section title="Audio">
          {audioTracks.map((t) => (
            <MenuItem key={t.id} selected={audioTrack === t.id} onClick={() => onAudio(t.id)} label={t.label} />
          ))}
        </Section>
      )}
      {subtitleTracks.length > 0 && (
        <Section title="Subtitles">
          <MenuItem selected={subtitleTrack === -1} onClick={() => onSubtitle(-1)} label="Off" />
          {subtitleTracks.map((t) => (
            <MenuItem key={t.id} selected={subtitleTrack === t.id} onClick={() => onSubtitle(t.id)} label={t.label} />
          ))}
        </Section>
      )}
      {seekable && (
        <Section title="Speed">
          {SPEEDS.map((r) => (
            <MenuItem key={r} selected={speed === r} onClick={() => onSpeed(r)} label={r === 1 ? "Normal" : `${r}×`} />
          ))}
        </Section>
      )}
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="border-b border-white/10 py-1 last:border-0">
      <p className="px-3 py-1 text-[11px] font-semibold uppercase tracking-wide text-white/40">{title}</p>
      {children}
    </div>
  );
}

function MenuItem({ selected, onClick, label }: { selected: boolean; onClick: () => void; label: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-2 px-3 py-1.5 text-left transition-colors hover:bg-white/10"
    >
      <Check className={cn("size-3.5 shrink-0", selected ? "text-accent-strong" : "opacity-0")} aria-hidden="true" />
      <span className="truncate">{label}</span>
    </button>
  );
}

/** A round translucent control button. */
function IconBtn({ label, onClick, children }: { label: string; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
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
