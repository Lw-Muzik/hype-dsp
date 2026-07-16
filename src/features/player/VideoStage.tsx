import { useEffect, useRef, useState } from "react";
import { CircleAlert, Loader2 } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { ipcErrorMessage, ytmusicVideoUrl } from "@/lib/ipc";
import { syncAction } from "@/features/player/videoSync";

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

  // Resolve on demand: this costs a yt-dlp spawn, so it happens when video is
  // switched on, not for every track that might have one.
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
    <Stage>
      <video
        ref={ref}
        src={url}
        muted
        playsInline
        // The engine drives position; the element must never seek itself.
        controls={false}
        onError={() => setError("This video couldn't be played.")}
        className="size-full object-contain"
      />
    </Stage>
  );
}

function Stage({ children }: { children: React.ReactNode }) {
  return (
    <div className="grid aspect-video w-full place-items-center overflow-hidden rounded-card bg-black ring-1 ring-border">
      {children}
    </div>
  );
}
