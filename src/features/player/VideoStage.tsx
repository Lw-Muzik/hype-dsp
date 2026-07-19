import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  CircleAlert,
  Loader2,
  Maximize,
  Minimize,
  Pause,
  Play,
  SkipBack,
  SkipForward,
} from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { ipcErrorMessage, ytmusicVideoUrl } from "@/lib/ipc";
import { syncAction } from "@/features/player/videoSync";
import { SeekBar } from "@/features/player/SeekBar";
import { formatTime } from "@/lib/format";
import { cn } from "@/lib/cn";

/**
 * The music video for the current track, as a picture only.
 *
 * `muted` is not a default to be overridden — it is the design. The element is
 * fed a **video-only** rendition, so it has no audio track to play even if it
 * were unmuted: the sound is the engine's, through the whole enhancement chain,
 * and that can't be bypassed by accident here.
 *
 * The URL goes through the loopback proxy because the element can reach neither
 * googlevideo's origin (the CSP forbids it) nor its User-Agent requirement (no
 * element can set request headers).
 *
 * Every failure below is silent to playback. A video that won't resolve, won't
 * load, or won't decode leaves the audio exactly as it was — the picture is the
 * optional half.
 */
export function VideoStage({ videoId }: { videoId: string }) {
  const ref = useRef<HTMLVideoElement>(null);
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fullscreen, setFullscreen] = useState(false);

  // The live fullscreen value for the unmount cleanup, which otherwise closes
  // over the initial `false` and would leave the OS window stuck fullscreen.
  const fullscreenRef = useRef(false);
  fullscreenRef.current = fullscreen;

  // Fullscreen is an in-app overlay + the OS window, NOT the DOM Fullscreen API:
  // macOS WKWebView disables `element.requestFullscreen`, so it silently does
  // nothing there (it only works on Windows' WebView2). This mirrors the
  // visualizer and the TV player. The OS window is a bonus for immersion; the
  // overlay alone already covers the app.
  const toggleFullscreen = useCallback(() => {
    setFullscreen((on) => {
      const next = !on;
      void getCurrentWindow().setFullscreen(next).catch(() => {});
      return next;
    });
  }, []);

  // Esc leaves fullscreen, the universal expectation. Only while it's on.
  useEffect(() => {
    if (!fullscreen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") toggleFullscreen();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fullscreen, toggleFullscreen]);

  // Leaving the video (track has no footage, panel closed) must not strand the
  // OS window in fullscreen with no way back to it.
  useEffect(() => {
    return () => {
      if (fullscreenRef.current) void getCurrentWindow().setFullscreen(false).catch(() => {});
    };
  }, []);

  // Resolve the url for the picture. Usually a cache hit now — the engine
  // prefetches every video-capable track's url the moment it starts playing, so
  // by the time this mounts the resolve is already warm and the picture appears
  // at once. Cold only if the video tab is opened within the ~5s prefetch, or
  // the prefetch failed; either way this pays the yt-dlp spawn itself.
  useEffect(() => {
    let cancelled = false;
    setUrl(null);
    setError(null);
    ytmusicVideoUrl(videoId)
      .then((u) => {
        if (!cancelled) setUrl(u);
      })
      .catch((e) => {
        if (!cancelled) setError(ipcErrorMessage(e));
      });
    return () => {
      cancelled = true;
    };
  }, [videoId]);

  // Follow the engine. Subscribing rather than reading in a render keeps this
  // out of React's cycle: the correction is a side effect on a DOM node, and the
  // engine ticks ~10×/s.
  useEffect(() => {
    const apply = () => {
      const video = ref.current;
      if (!video) return;
      const s = useEngineStore.getState();
      const { seekTo, setPaused } = syncAction({
        enginePos: s.positionSecs,
        videoPos: video.currentTime,
        paused: s.paused || !s.playing,
        videoPaused: video.paused,
        // HAVE_CURRENT_DATA — anything less and its clock means nothing yet.
        ready: video.readyState >= 2,
      });
      if (setPaused !== null) {
        // `play()` rejects if it's interrupted (a seek, a source change). That's
        // routine, and it must never surface as a playback error.
        if (setPaused) video.pause();
        else void video.play().catch(() => {});
      }
      if (seekTo !== null) video.currentTime = seekTo;
    };
    apply();
    return useEngineStore.subscribe(apply);
  }, [url]);

  if (error) {
    return (
      <Stage>
        <div className="flex flex-col items-center gap-2 text-center">
          <CircleAlert className="size-6 text-danger" aria-hidden="true" />
          <p className="max-w-xs text-xs text-text-muted">{error}</p>
          <p className="text-[10px] text-text-faint">Audio is unaffected.</p>
        </div>
      </Stage>
    );
  }

  if (!url) {
    return (
      <Stage>
        <Loader2 className="size-5 animate-spin text-text-faint" aria-hidden="true" />
      </Stage>
    );
  }

  // Fullscreen is a CSS change on this same element, never a move in the DOM.
  // An earlier version portalled the stage to `document.body` on fullscreen,
  // which **remounted the `<video>`** — a fresh element that re-fetched and
  // re-buffered the stream each toggle, so entering fullscreen stalled on a
  // reload. The portal was there to escape a containing-block trap, but this
  // component's ancestors (the app shell, the right sidebar) carry no
  // `transform`/`filter`/`will-change`, so a plain `fixed inset-0` already
  // escapes to the viewport. Keeping the element in place means fullscreen is
  // instant: the picture is already decoded and playing, it just fills the
  // screen.
  return (
    <Stage fullscreen={fullscreen}>
      <video
        ref={ref}
        src={url}
        muted
        playsInline
        // Native controls would be the wrong instrument entirely: they seek the
        // element, and the element's clock is not the truth — correcting it is
        // what `syncAction` exists to do, so a native scrub would be undone
        // within a tick. They also offer a volume slider that does nothing, on a
        // muted, video-only rendition. `VideoControls` drives the engine instead,
        // and the picture follows from that, the same way it always does.
        controls={false}
        onError={() => setError("This video couldn't be played.")}
        className="size-full object-contain"
      />
      <VideoControls fullscreen={fullscreen} onToggleFullscreen={toggleFullscreen} />
    </Stage>
  );
}

/**
 * Transport laid over the picture, wired to the **engine**, never to the
 * element.
 *
 * Every control here is the one from the main transport bar, in reach of
 * someone watching. Pressing play moves the audio, and the picture catches up on
 * the next tick — which is the same path a click on the main bar takes, so there
 * is no second way to control playback to keep consistent with the first.
 *
 * Fullscreen is the exception, and the only thing here that touches the DOM: it
 * belongs to the stage, not to playback, and it takes the whole stage rather
 * than the `<video>` so these controls come along.
 */
function VideoControls({
  fullscreen,
  onToggleFullscreen,
}: {
  fullscreen: boolean;
  onToggleFullscreen: () => void;
}) {
  // Subscribed here rather than in `VideoStage` so a ~10×/s position tick
  // re-renders this strip alone, and never the element showing the video.
  const playing = useEngineStore((s) => s.playing);
  const paused = useEngineStore((s) => s.paused);
  const positionSecs = useEngineStore((s) => s.positionSecs);
  const durationSecs = useEngineStore((s) => s.durationSecs);
  const seekable = useEngineStore((s) => s.seekable);
  const queueLength = useEngineStore((s) => s.queue.length);
  const queueIndex = useEngineStore((s) => s.queueIndex);
  const togglePause = useEngineStore((s) => s.togglePause);
  const next = useEngineStore((s) => s.next);
  const prev = useEngineStore((s) => s.prev);
  const seek = useEngineStore((s) => s.seek);

  const showPause = playing && !paused;
  const duration = durationSecs ?? 0;

  return (
    <div
      className={cn(
        "absolute inset-x-0 bottom-0 flex items-center gap-2 bg-gradient-to-t from-black/80 to-transparent px-3 pb-2 pt-8",
        // Out of the way while watching, back the moment it's wanted. Kept up
        // whenever the video isn't running, so a paused picture is never a dead
        // rectangle with no way out of it.
        "opacity-0 transition-opacity duration-150 focus-within:opacity-100 group-hover/stage:opacity-100",
        !showPause && "opacity-100",
      )}
    >
      <button
        type="button"
        aria-label="Previous track"
        onClick={prev}
        disabled={queueIndex <= 0}
        className={overlayBtn}
      >
        <SkipBack className="size-4" aria-hidden="true" />
      </button>
      <button
        type="button"
        aria-label={showPause ? "Pause" : "Play"}
        onClick={togglePause}
        className={overlayBtn}
      >
        {showPause ? (
          <Pause className="size-5" aria-hidden="true" />
        ) : (
          <Play className="size-5" aria-hidden="true" />
        )}
      </button>
      <button
        type="button"
        aria-label="Next track"
        onClick={next}
        disabled={queueIndex < 0 || queueIndex + 1 >= queueLength}
        className={overlayBtn}
      >
        <SkipForward className="size-4" aria-hidden="true" />
      </button>

      <span className="w-9 text-right text-[11px] tabular-nums text-white/70">
        {formatTime(positionSecs)}
      </span>
      <SeekBar
        position={positionSecs}
        duration={duration}
        seekable={seekable}
        onSeek={seek}
        className="flex-1"
      />
      <span className="w-9 text-[11px] tabular-nums text-white/70">
        {formatTime(durationSecs)}
      </span>

      <button
        type="button"
        aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
        onClick={onToggleFullscreen}
        className={overlayBtn}
      >
        {fullscreen ? (
          <Minimize className="size-4" aria-hidden="true" />
        ) : (
          <Maximize className="size-4" aria-hidden="true" />
        )}
      </button>
    </div>
  );
}

const overlayBtn =
  "flex size-8 shrink-0 items-center justify-center rounded-full text-white/80 transition-colors hover:bg-white/15 hover:text-white disabled:pointer-events-none disabled:opacity-30";

function Stage({
  children,
  fullscreen = false,
}: {
  children: React.ReactNode;
  fullscreen?: boolean;
}) {
  return (
    <div
      className={cn(
        "group/stage grid w-full place-items-center overflow-hidden bg-black",
        // Fullscreen breaks this element out of the sidebar to cover the
        // viewport — `fixed inset-0`, not a portal. No ancestor is a containing
        // block (no transform/filter in the chain), so fixed escapes cleanly,
        // and staying in place is what keeps the picture from reloading. Drop
        // the 16:9 box and card chrome, which would letterbox inside the screen.
        fullscreen
          ? "fixed inset-0 z-[100]"
          : "relative aspect-video rounded-card ring-1 ring-border",
      )}
    >
      {children}
    </div>
  );
}
