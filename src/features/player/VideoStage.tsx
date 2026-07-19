import { useCallback, useEffect, useRef, useState } from "react";
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
  const stageRef = useRef<HTMLDivElement>(null);
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Resolve on demand: this costs a yt-dlp spawn on a cold cache, so it happens
  // when video is switched on, not for every track that might have one. The
  // backend caches the result for the url's lifetime, so re-opening this tab or
  // re-mounting the element is free.
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

  return (
    <Stage ref={stageRef}>
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
      <VideoControls stageRef={stageRef} />
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
function VideoControls({ stageRef }: { stageRef: React.RefObject<HTMLDivElement | null> }) {
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

  const [fullscreen, setFullscreen] = useState(false);

  // The browser owns this state — Esc and the system chrome can both leave
  // fullscreen without going through our button, so read it rather than track it.
  useEffect(() => {
    const onChange = () => setFullscreen(document.fullscreenElement === stageRef.current);
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, [stageRef]);

  const toggleFullscreen = useCallback(() => {
    const stage = stageRef.current;
    if (!stage) return;
    // Rejects when the gesture isn't trusted or the element can't be promoted;
    // either way the picture keeps playing exactly as it was.
    if (document.fullscreenElement === stage) void document.exitFullscreen().catch(() => {});
    else void stage.requestFullscreen().catch(() => {});
  }, [stageRef]);

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
        onClick={toggleFullscreen}
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
  ref,
}: {
  children: React.ReactNode;
  ref?: React.Ref<HTMLDivElement>;
}) {
  return (
    <div
      ref={ref}
      className={cn(
        "group/stage relative grid aspect-video w-full place-items-center overflow-hidden rounded-card bg-black ring-1 ring-border",
        // Fullscreen hands this element the whole screen, which a fixed 16:9 box
        // would then letterbox *inside* — black bars around a black stage. Drop
        // the ratio and the card chrome for as long as it's promoted.
        "[&:fullscreen]:aspect-auto [&:fullscreen]:h-full [&:fullscreen]:rounded-none [&:fullscreen]:ring-0",
      )}
    >
      {children}
    </div>
  );
}
