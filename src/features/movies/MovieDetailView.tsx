import { useState } from "react";
import { ChevronLeft, CircleAlert, Play as PlayIcon, RotateCw } from "lucide-react";
import { Button } from "@/components/Button";
import { Segmented } from "@/components/Segmented";
import { useMoviesStore } from "@/stores/movies";
import { MoviePoster } from "./MoviePoster";
import { MoviePlayer } from "./MoviePlayer";
import { YoutubeMoviePlayer } from "./YoutubeMoviePlayer";
import { availableQualities, selectStream, type Quality } from "./streamSelect";
import { formatDurationLabel } from "./duration";

/**
 * A movie's detail screen. Branches on `opened.source` first thing: Archive
 * titles keep the original poster/synopsis/SD-HD-toggle screen and play
 * through `MoviePlayer` (a direct MP4 the app streams itself); YouTube titles
 * have no per-item resolve step — the channel listing already carried
 * everything they need — and play through `YoutubeMoviePlayer` (the official
 * iframe embed) with no quality toggle at all, since YouTube handles
 * rendition switching itself.
 *
 * Mounted with a source-aware key by `MoviesView`, so switching movies always
 * starts from a clean "not playing" state rather than carrying the previous
 * movie's over.
 */
export function MovieDetailView() {
  const opened = useMoviesStore((s) => s.opened);
  const detail = useMoviesStore((s) => s.detail);
  const detailLoad = useMoviesStore((s) => s.detailLoad);
  const detailError = useMoviesStore((s) => s.detailError);
  const close = useMoviesStore((s) => s.close);
  const retryDetail = useMoviesStore((s) => s.retryDetail);

  const [quality, setQuality] = useState<Quality>("SD");
  const [playing, setPlaying] = useState(false);

  if (!opened) return null;

  if (opened.source === "youtube") {
    if (playing) {
      return (
        <div className="flex flex-col gap-3 pb-4">
          <BackButton onClick={() => setPlaying(false)} label="Back to details" />
          <YoutubeMoviePlayer title={opened.title} videoId={opened.videoId} onClose={() => setPlaying(false)} />
        </div>
      );
    }
    return (
      <div className="flex flex-col gap-4 pb-4">
        <BackButton onClick={close} label="Movies" />

        <div className="flex flex-col gap-5 sm:flex-row">
          <MoviePoster
            src={opened.posterUrl}
            title={opened.title}
            className="aspect-video w-full shrink-0 rounded-card sm:w-64"
          />

          <div className="flex min-w-0 flex-1 flex-col gap-3">
            <div>
              <h2 className="text-xl font-semibold">{opened.title}</h2>
              <p className="text-sm text-text-muted">
                {opened.channelTitle ?? "YouTube"}
                {opened.durationSec > 0 ? ` · ${formatDurationLabel(opened.durationSec)}` : ""}
              </p>
            </div>

            <p className="text-sm text-text-faint">
              Plays through the official YouTube player, ads included — the licensed way this
              distributor's full movies can be watched here.
            </p>

            <div className="flex flex-wrap items-center gap-3 pt-1">
              <Button variant="primary" onClick={() => setPlaying(true)}>
                <PlayIcon className="size-4" aria-hidden="true" />
                Play
              </Button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  const title = detail?.title ?? opened.title;
  const year = detail?.year ?? opened.year;
  const poster = detail?.posterUrl ?? opened.posterUrl;
  const streams = detail?.streams ?? [];
  const qualities = availableQualities(streams);
  const stream = selectStream(streams, quality);

  if (playing && stream) {
    return (
      <div className="flex flex-col gap-3 pb-4">
        <BackButton onClick={() => setPlaying(false)} label="Back to details" />
        <MoviePlayer
          title={title}
          poster={poster}
          streams={streams}
          initialQuality={quality}
          onClose={() => setPlaying(false)}
        />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 pb-4">
      <BackButton onClick={close} label="Movies" />

      <div className="flex flex-col gap-5 sm:flex-row">
        <MoviePoster
          src={poster}
          title={title}
          className="aspect-[2/3] w-full shrink-0 rounded-card sm:w-48"
        />

        <div className="flex min-w-0 flex-1 flex-col gap-3">
          <div>
            <h2 className="text-xl font-semibold">{title}</h2>
            <p className="text-sm text-text-muted">{year ?? "Year unknown"}</p>
          </div>

          {detailLoad === "loading" ? (
            <div className="flex flex-col gap-2">
              <div className="h-3 w-full animate-pulse rounded bg-surface-overlay" />
              <div className="h-3 w-full animate-pulse rounded bg-surface-overlay" />
              <div className="h-3 w-2/3 animate-pulse rounded bg-surface-overlay" />
            </div>
          ) : detailLoad === "error" ? (
            <div className="flex flex-col items-start gap-2 rounded-control border border-danger/30 bg-danger/10 p-3">
              <p className="flex items-center gap-2 text-sm text-danger">
                <CircleAlert className="size-4 shrink-0" aria-hidden="true" />
                {detailError ?? "Couldn't load this film's details."}
              </p>
              <Button variant="secondary" onClick={retryDetail}>
                <RotateCw className="size-4" aria-hidden="true" />
                Retry
              </Button>
            </div>
          ) : (
            <p className="whitespace-pre-line text-sm text-text-muted">
              {detail?.description || "No synopsis available."}
            </p>
          )}

          {detailLoad === "ready" && (
            <div className="flex flex-wrap items-center gap-3 pt-1">
              <Button variant="primary" disabled={!stream} onClick={() => setPlaying(true)}>
                <PlayIcon className="size-4" aria-hidden="true" />
                Play
              </Button>

              {qualities.length > 1 && (
                <Segmented
                  items={qualities.map((q) => ({ value: q, label: q }))}
                  value={quality}
                  onChange={setQuality}
                  label="Video quality"
                  className="w-auto"
                />
              )}

              {!stream && (
                <p className="text-xs text-text-faint">No playable version of this film is available.</p>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function BackButton({ onClick, label }: { onClick: () => void; label: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center gap-1 self-start text-sm text-text-muted transition-colors hover:text-text"
    >
      <ChevronLeft className="size-4" aria-hidden="true" />
      {label}
    </button>
  );
}
