import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { AlertTriangle, ExternalLink, Loader2, Maximize, Minimize, X } from "lucide-react";
import { useEngineStore } from "@/stores/engine";
import { openExternalUrl } from "@/lib/ipc";
import { cn } from "@/lib/cn";

type Status = "loading" | "ready" | "error";

/**
 * Minimal ambient typing for the one thing this file calls on the official
 * YouTube IFrame Player API — not a stand-in for `@types/youtube`, which this
 * project doesn't depend on.
 */
interface YtPlayerInstance {
  destroy: () => void;
}
interface YtPlayerCtor {
  new (
    el: HTMLElement,
    opts: {
      videoId: string;
      host?: string;
      width?: string | number;
      height?: string | number;
      playerVars?: Record<string, number | string>;
      events?: {
        onReady?: () => void;
        onError?: (e: { data: number }) => void;
      };
    },
  ): YtPlayerInstance;
}
declare global {
  interface Window {
    YT?: { Player: YtPlayerCtor };
    onYouTubeIframeAPIReady?: () => void;
  }
}

const API_URL = "https://www.youtube.com/iframe_api";
/** `youtube-nocookie.com` is the privacy-enhanced embed domain — no YouTube
 *  cookies are set until the visitor actually presses play. */
const EMBED_HOST = "https://www.youtube-nocookie.com";

/** Loads the IFrame API once per app session. Guards two things: a second
 *  `<script>` tag (this component can mount more than once across the app's
 *  lifetime, and React StrictMode double-invokes effects in dev), and
 *  clobbering another caller's `onYouTubeIframeAPIReady` — YouTube's API only
 *  ever looks for one such global, so a second definition must chain to the
 *  first rather than replace it. */
let apiPromise: Promise<void> | null = null;
function loadIframeApi(): Promise<void> {
  if (window.YT?.Player) return Promise.resolve();
  if (apiPromise) return apiPromise;
  apiPromise = new Promise((resolve) => {
    const previous = window.onYouTubeIframeAPIReady;
    window.onYouTubeIframeAPIReady = () => {
      previous?.();
      resolve();
    };
    if (!document.querySelector(`script[src="${API_URL}"]`)) {
      const tag = document.createElement("script");
      tag.src = API_URL;
      document.head.appendChild(tag);
    }
  });
  return apiPromise;
}

/**
 * The in-app YouTube movie player — the official IFrame Player API, not the
 * app's own stream path. Every error the API reports (2/5 = bad request or an
 * HTML5 player fault, 100/101/150 = removed, private, or embedding disabled
 * by the owner) means the same thing here: this title can't play in an
 * embed, never something to retry, so every code maps to the same fallback.
 *
 * Unlike `MoviePlayer` (a direct MP4 this app streams and fully controls),
 * this component owns almost none of the player surface: YouTube runs its
 * own ads and renders its own controls inside the iframe, and the Required
 * Minimum Functionality rules forbid any clickable overlay on top of that
 * area. So the title bar / close / fullscreen controls here live *outside*
 * the video rect (a row above it), never layered on top of it — the only
 * exception is the loading spinner (shown before the iframe has any content
 * to cover) and the error fallback (shown only once the embed has already
 * failed, replacing the area rather than covering a live one).
 *
 * The mount `<div>` is rendered unconditionally for this component's entire
 * lifetime and never wrapped in a condition that would remove it from the
 * tree: the IFrame API replaces that DOM node with its own `<iframe>` outside
 * React's knowledge, and if React ever tried to remove *that specific node*
 * on its own (rather than tearing down the whole subtree, which is safe) it
 * would throw — the node it's holding a reference to is no longer attached
 * where it thinks it is.
 */
export function YoutubeMoviePlayer({
  title,
  videoId,
  onClose,
}: {
  title: string;
  videoId: string;
  onClose: () => void;
}) {
  const mountRef = useRef<HTMLDivElement>(null);
  const playerRef = useRef<YtPlayerInstance | null>(null);
  const stopEngine = useEngineStore((s) => s.stop);

  const [status, setStatus] = useState<Status>("loading");
  const [fullscreen, setFullscreen] = useState(false);
  // The live fullscreen value for the unmount cleanup, which would otherwise
  // close over the initial `false` and leave the OS window stuck fullscreen.
  const fullscreenRef = useRef(false);
  fullscreenRef.current = fullscreen;

  // Own the audio route the instant a movie starts: the embed has its own
  // audio (and ads) and must never fight the DSP engine for the output
  // device. Same one-per-mount contract as `MoviePlayer`/`TvPlayer`.
  useEffect(() => {
    void stopEngine();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");

    loadIframeApi().then(() => {
      if (cancelled || !mountRef.current || !window.YT) return;
      playerRef.current = new window.YT.Player(mountRef.current, {
        videoId,
        host: EMBED_HOST,
        width: "100%",
        height: "100%",
        // `rel:0`/`modestbranding:1` trim the surrounding chrome; the
        // player's own fullscreen control (`fs`) is left at its default of
        // on — Required Minimum Functionality forbids turning it off. No
        // `autoplay`: this app never plays audio/video the user hasn't asked
        // for, and pressing this screen's own Play button already was that
        // ask, so this is a deliberate no-op rather than a broken promise.
        playerVars: { rel: 0, modestbranding: 1 },
        events: {
          onReady: () => {
            if (!cancelled) setStatus("ready");
          },
          onError: () => {
            if (!cancelled) setStatus("error");
          },
        },
      });
    });

    return () => {
      cancelled = true;
      playerRef.current?.destroy();
      playerRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [videoId]);

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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && fullscreen) toggleFullscreen();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fullscreen, toggleFullscreen]);

  const openOnYoutube = useCallback(() => {
    void openExternalUrl(`https://www.youtube.com/watch?v=${videoId}`);
  }, [videoId]);

  return (
    <div className={cn("flex flex-col gap-2", fullscreen && "fixed inset-0 z-[100] bg-black p-3")}>
      {/* The shell — title, fullscreen, close — sits above the player rect,
          never on top of it. */}
      <div className="flex items-center gap-2 px-0.5">
        <p className={cn("min-w-0 flex-1 truncate text-sm font-semibold", fullscreen && "text-white")}>
          {title}
        </p>
        <button
          type="button"
          aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
          onClick={toggleFullscreen}
          className={cn(shellBtn, fullscreen && shellBtnDark)}
        >
          {fullscreen ? <Minimize className="size-4" aria-hidden="true" /> : <Maximize className="size-4" aria-hidden="true" />}
        </button>
        <button
          type="button"
          aria-label="Close player"
          onClick={onClose}
          className={cn(shellBtn, fullscreen && shellBtnDark)}
        >
          <X className="size-4" aria-hidden="true" />
        </button>
      </div>

      <div
        className={cn(
          "relative w-full overflow-hidden rounded-card bg-black ring-1 ring-border",
          fullscreen ? "flex-1" : "aspect-video",
        )}
      >
        {/* Always mounted for this component's whole lifetime — see the
            class doc comment above for why this must never be conditionally
            removed while the rest of the tree stays up. */}
        <div ref={mountRef} className="size-full" />

        {status === "loading" && (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center bg-black">
            <Loader2 className="size-9 animate-spin text-white/80" aria-hidden="true" />
          </div>
        )}

        {status === "error" && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-black px-6 text-center">
            <AlertTriangle className="size-7 text-warning" aria-hidden="true" />
            <p className="max-w-xs text-sm text-white/80">
              This title can't be played here — the distributor doesn't allow it to embed.
            </p>
            <button
              type="button"
              onClick={openOnYoutube}
              className="flex items-center gap-2 rounded-control bg-white/10 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-white/20"
            >
              <ExternalLink className="size-4" aria-hidden="true" />
              Open on YouTube
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

const shellBtn =
  "flex size-8 shrink-0 items-center justify-center rounded-full text-text-muted transition-colors hover:bg-surface-overlay hover:text-text";
const shellBtnDark = "text-white/80 hover:bg-white/15 hover:text-white";
