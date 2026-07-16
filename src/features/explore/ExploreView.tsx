import { useEffect } from "react";
import { ChevronLeft, CircleAlert, Disc3, ListMusic, Loader2, RotateCw, SquarePlay } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/Button";
import { Artwork } from "@/features/player/Artwork";
import { useExploreStore } from "@/stores/explore";
import { useUiStore } from "@/stores/ui";
import type { ExploreItem, ExploreShelf } from "@/lib/types";
import { cn } from "@/lib/cn";

/**
 * Explore — YouTube Music's own catalog.
 *
 * Two screens: a picker of the mood/genre categories, and one category's
 * shelves. Everything is fetched on click and nothing is kept: this is the live
 * catalog, and a cached copy of it would just be a worse Library.
 */
export function ExploreView() {
  const route = routeById("explore");
  const setRoute = useUiStore((s) => s.setRoute);

  const signedIn = useExploreStore((s) => s.signedIn);
  const sections = useExploreStore((s) => s.sections);
  const sectionsLoad = useExploreStore((s) => s.sectionsLoad);
  const sectionsError = useExploreStore((s) => s.sectionsError);
  const selected = useExploreStore((s) => s.selected);
  const shelves = useExploreStore((s) => s.shelves);
  const pageLoad = useExploreStore((s) => s.pageLoad);
  const pageError = useExploreStore((s) => s.pageError);
  const ensureCategories = useExploreStore((s) => s.ensureCategories);
  const select = useExploreStore((s) => s.select);
  const clear = useExploreStore((s) => s.clear);
  const retry = useExploreStore((s) => s.retry);

  useEffect(() => {
    ensureCategories();
  }, [ensureCategories]);

  const error = selected ? pageError : sectionsError;
  const loading = selected ? pageLoad === "loading" : sectionsLoad === "loading";

  return (
    <div className="mx-auto flex h-full w-full max-w-5xl flex-col gap-4">
      <PageHeader
        icon={route.icon}
        title={selected ? selected.title : route.label}
        subtitle={selected ? "Playlists and albums from YouTube Music." : route.tagline}
      />

      {selected && (
        <button
          type="button"
          onClick={clear}
          className="flex items-center gap-1 self-start text-sm text-text-muted transition-colors hover:text-text"
        >
          <ChevronLeft className="size-4" aria-hidden="true" />
          All categories
        </button>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {sectionsLoad === "ready" && !signedIn ? (
          <Centered
            icon={SquarePlay}
            title="Not signed in to YouTube Music"
            body="Sign in from Settings to browse YouTube's playlists and albums here."
            action={
              <Button variant="primary" onClick={() => setRoute("settings")}>
                <ListMusic className="size-4" aria-hidden="true" />
                Sign in from Settings
              </Button>
            }
          />
        ) : error ? (
          <Centered
            icon={CircleAlert}
            danger
            title={selected ? "Couldn't load this category" : "Couldn't load Explore"}
            body={error}
            action={
              <Button variant="primary" onClick={retry}>
                <RotateCw className="size-4" aria-hidden="true" />
                Retry
              </Button>
            }
          />
        ) : loading ? (
          <div className="flex items-center justify-center gap-2 py-16 text-sm text-text-muted">
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            {selected ? `Loading ${selected.title}…` : "Loading Explore…"}
          </div>
        ) : selected ? (
          <div className="flex flex-col gap-8 pb-4">
            {shelves.map((shelf) => (
              <Shelf key={shelf.title} shelf={shelf} />
            ))}
          </div>
        ) : (
          <div className="flex flex-col gap-6 pb-4">
            {sections.map((section) => (
              <section key={section.title} className="flex flex-col gap-3">
                <h2 className="text-sm font-medium text-text-muted">{section.title}</h2>
                <div className="flex flex-wrap gap-2">
                  {section.categories.map((c) => (
                    <button
                      key={c.params}
                      type="button"
                      onClick={() => select(c)}
                      className="rounded-control border border-border bg-surface-raised px-4 py-2 text-sm transition-colors hover:border-border-strong hover:bg-surface-overlay"
                    >
                      {c.title}
                    </button>
                  ))}
                </div>
              </section>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function Shelf({ shelf }: { shelf: ExploreShelf }) {
  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-sm font-medium text-text-muted">{shelf.title}</h2>
      {/* Horizontal, like YouTube's own carousels — a shelf can hold ~100 items
          and stacking them all would bury the next shelf. */}
      <div className="flex gap-3 overflow-x-auto pb-2">
        {shelf.items.map((item) => (
          <Tile key={`${item.kind}:${item.id}`} item={item} />
        ))}
      </div>
    </section>
  );
}

function Tile({ item }: { item: ExploreItem }) {
  const play = useExploreStore((s) => s.play);
  const opening = useExploreStore((s) => s.opening);
  const busy = opening === item.id;
  const Icon = item.kind === "album" ? Disc3 : ListMusic;

  return (
    <button
      type="button"
      disabled={busy}
      onClick={() => void play(item)}
      title={item.subtitle ? `${item.title} — ${item.subtitle}` : item.title}
      className={cn(
        "group flex w-40 shrink-0 flex-col gap-2 rounded-lg p-2 text-left transition-colors",
        "hover:bg-surface-raised disabled:opacity-60",
      )}
    >
      <div className="relative">
        <Artwork
          art={{ key: item.id, source: "ytmusic", cover: item.thumbnail }}
          seed={item.id}
          label={item.title}
          className="aspect-square w-full"
        />
        <span className="absolute bottom-1 right-1 grid size-6 place-items-center rounded-full bg-surface-overlay/90 opacity-0 transition-opacity group-hover:opacity-100">
          {busy ? (
            <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
          ) : (
            <Icon className="size-3.5" aria-hidden="true" />
          )}
        </span>
      </div>
      <div className="min-w-0">
        <p className="truncate text-sm font-medium">{item.title}</p>
        {item.subtitle && (
          <p className="truncate text-xs text-text-muted">{item.subtitle}</p>
        )}
      </div>
    </button>
  );
}

function Centered({
  icon: Icon,
  title,
  body,
  action,
  danger = false,
}: {
  icon: typeof CircleAlert;
  title: string;
  body: string;
  action?: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 py-16 text-center">
      <div
        className={cn(
          "grid size-14 place-items-center rounded-2xl ring-1",
          danger ? "bg-danger/10 ring-danger/30" : "bg-surface-raised ring-border",
        )}
      >
        <Icon
          className={cn("size-7", danger ? "text-danger" : "text-text-faint")}
          aria-hidden="true"
        />
      </div>
      <div>
        <p className="text-base font-medium">{title}</p>
        <p className="mt-1 max-w-sm text-sm text-text-muted">{body}</p>
      </div>
      {action}
    </div>
  );
}
