import { create } from "zustand";
import {
  cloudAllAudio,
  cloudCachedMetadata,
  cloudStatus,
  cloudTrackMetadata,
  libraryAvailableCount,
  libraryCount,
  libraryListPage,
  linkLibrary,
  linkPaired,
} from "@/lib/ipc";
import { cloudItem, localItem, phoneItem } from "@/stores/engine";
import type { QueueItem } from "@/stores/engine";
import type {
  CloudAudioPage,
  CloudEntry,
  CloudProvider,
  CloudTrackMeta,
  LibraryPage,
  LibraryTrack,
} from "@/lib/types";

/** A browsable track from any source, ready to enqueue (it's a `QueueItem`). */
export interface MusicTrack extends QueueItem {
  source: "local" | "phone" | "cloud";
  /** Unique across sources, for React keys and highlight matching. */
  uid: string;
  /** Genre from tags (local only for now), for the Genres facet. */
  genre: string | null;
  /** Folder/grouping label for the Folders facet. */
  folder: string | null;
  /** Local file path for lazy embedded artwork (null for phone/cloud). */
  artPath: string | null;
  /** Pre-resolved cover (a `data:` URI) — cloud tracks after metadata preload. */
  cover: string | null;
}

/**
 * A source's load lifecycle. `ready` (success) and `error` both mean "we've
 * finished trying, so `connected`/`count` are trustworthy" — the UI only shows
 * a connect prompt or empty state once a source leaves `idle`/`loading`.
 */
export type LoadStatus = "idle" | "loading" | "ready" | "error";

/** Per-source availability + counts, for the source filter UI. */
export interface SourceState {
  /** Whether the source is reachable/connected (local is always true). */
  connected: boolean;
  loading: boolean;
  /** A load has finished at least once → connected/count can be trusted. */
  ready: boolean;
  /** Tracks loaded so far (climbs as a large library streams in). */
  count: number;
  /** Total tracks expected while loading, for a progress fraction (0 if unknown). */
  total: number;
}

/** The immediate parent folder name of a file path (for the Folders facet). */
function parentFolder(path: string): string | null {
  const norm = path.replace(/\\/g, "/").replace(/\/+$/, "");
  const parts = norm.split("/").filter(Boolean);
  return parts.length >= 2 ? parts[parts.length - 2]! : null;
}

/** Map one stored library row into a browsable `MusicTrack`. */
function mapLocalTrack(t: LibraryTrack): MusicTrack {
  return {
    ...localItem(t),
    source: "local" as const,
    uid: `local:${t.path}`,
    genre: t.genre,
    folder: parentFolder(t.path),
    artPath: t.path,
    cover: null,
  };
}

/**
 * Yield to the event loop so the browser can paint and process input between
 * work chunks. A `MessageChannel` task has none of `setTimeout`'s ~4ms clamp;
 * fall back to `setTimeout` where it isn't available.
 */
function yieldToMain(): Promise<void> {
  return new Promise((resolve) => {
    if (typeof MessageChannel !== "undefined") {
      const ch = new MessageChannel();
      ch.port1.onmessage = () => resolve();
      ch.port2.postMessage(undefined);
    } else {
      setTimeout(resolve, 0);
    }
  });
}

// How many tracks to pull per IPC page, and how often to push the growing list
// to the UI. A page is a bounded parse (~1000 rows ≈ a few ms); publishing on a
// time budget (not per page) keeps the O(n) consumer derivations down to a few
// per second instead of once per page, so the load stays smooth at any size.
const LOCAL_PAGE_SIZE = 1000;
const LOCAL_PUBLISH_INTERVAL_MS = 200;

/** Injectable dependencies for {@link loadLocalPaged} (pure + unit-testable). */
export interface LocalPagedDeps {
  /** Total track count (best-effort, for a progress fraction). */
  fetchCount: () => Promise<number>;
  /** Fetch one ordered page (reachable tracks only); `scanned` < `limit` means
   *  end-of-list, and is what advances the offset (so hidden rows don't stall). */
  fetchPage: (offset: number, limit: number) => Promise<LibraryPage>;
  /** Push the accumulated tracks (a fresh array) + known total to the store. */
  publish: (tracks: MusicTrack[], total: number) => void;
  /** Yield to the event loop between pages. */
  yieldToLoop: () => Promise<void>;
  /** True once this load was superseded (invalidate/reload) → stop now. */
  isStale: () => boolean;
  /** Monotonic clock (ms) for throttling publishes; injectable for tests. */
  now: () => number;
  pageSize?: number;
  publishIntervalMs?: number;
}

/**
 * Drive an incremental, non-blocking local-library load: page tracks in, map
 * them cheaply (O(1) per track), publish to the UI on a time budget, and yield
 * to the event loop between pages. The main thread never parses or maps the
 * whole library in one task, so the UI stays interactive even for a 1M-track
 * library (it populates progressively instead of freezing until done).
 *
 * Returns `true` if it ran to completion, `false` if it was cancelled mid-load
 * (so the caller only flips to "ready" on a genuine finish).
 */
export async function loadLocalPaged(deps: LocalPagedDeps): Promise<boolean> {
  const {
    fetchCount,
    fetchPage,
    publish,
    yieldToLoop,
    isStale,
    now,
    pageSize = LOCAL_PAGE_SIZE,
    publishIntervalMs = LOCAL_PUBLISH_INTERVAL_MS,
  } = deps;

  // Count is best-effort: it only drives a progress fraction, so a failure
  // here must not abort the load itself.
  let total = 0;
  try {
    total = await fetchCount();
  } catch {
    total = 0;
  }
  if (isStale()) return false;

  const acc: MusicTrack[] = [];
  let offset = 0;
  let publishedLen = -1; // force a publish after the first page
  let lastPublish = -Infinity;

  for (;;) {
    const page = await fetchPage(offset, pageSize);
    if (isStale()) return false;
    for (const row of page.tracks) acc.push(mapLocalTrack(row));
    // Advance by rows *scanned*, not rows returned: a page may hide unreachable
    // tracks (an unplugged drive), so returned length can be < scanned.
    offset += page.scanned;
    // The count can lag a concurrent scan; never claim fewer than we hold.
    total = Math.max(total, acc.length);

    const done = page.scanned < pageSize;
    // Publish when finished, or on the time budget — but only if there's
    // something new, so long stretches of hidden rows don't churn re-renders.
    const grew = acc.length > publishedLen;
    if ((done || now() - lastPublish >= publishIntervalMs) && (grew || done)) {
      publish(acc.slice(), total);
      publishedLen = acc.length;
      lastPublish = now();
    }
    if (done) return true;

    await yieldToLoop();
    if (isStale()) return false;
  }
}

/** Map a provider's flat, account-wide audio entries (all folders — mirrors the
 *  mobile app, so songs nested in subfolders are included) into browsable
 *  `MusicTrack`s. Pure: the listing is fetched (and cached) in `ensureCloud`. */
function entriesToTracks(
  provider: CloudProvider,
  entries: CloudEntry[],
): MusicTrack[] {
  return entries.map((e) => ({
    ...cloudItem(e),
    source: "cloud" as const,
    uid: `cloud:${provider}:${e.id}`,
    genre: null,
    folder: e.folder ?? null,
    artPath: null,
    cover: null,
  }));
}

// How many cloud files to read metadata for at once (the backend caches each
// after the first read, so this only bites on the first scan).
const CLOUD_META_CONCURRENCY = 4;

/**
 * Read embedded tags (title/artist/album + cover) for cloud tracks in the
 * background, like the mobile app's `CloudMetadataService`. Runs a few at a time
 * and reports each result through `onResult`; `isStale` lets the caller cancel
 * (e.g. on disconnect / reload). Backend-cached, so re-scans are cheap.
 */
async function preloadCloudMeta(
  tracks: MusicTrack[],
  onResult: (uid: string, meta: CloudTrackMeta) => void,
  isStale: () => boolean,
): Promise<void> {
  let next = 0;
  const worker = async () => {
    while (next < tracks.length && !isStale()) {
      const t = tracks[next++];
      const file = t?.cloud;
      if (!t || !file) continue;
      try {
        const meta = await cloudTrackMetadata(file.provider, file.id, file.name);
        if (meta && !isStale()) onResult(t.uid, meta);
      } catch {
        // Skip — a single failed read shouldn't stop the rest.
      }
    }
  };
  await Promise.all(
    Array.from({ length: CLOUD_META_CONCURRENCY }, () => worker()),
  );
}

interface MusicLibraryStore {
  local: MusicTrack[];
  phone: MusicTrack[];
  /** Cloud tracks as listed (filename titles); tags merge in via `cloudMeta`. */
  cloudBase: MusicTrack[];
  cloudMeta: Map<string, CloudTrackMeta>;

  localLoad: LoadStatus;
  /** The library `version` the local set was loaded for (re-fetch when it bumps). */
  localVersion: number;
  /** Total local tracks expected (for a load-progress fraction); `local.length`
   *  climbs toward it as pages stream in. */
  localTotal: number;
  phoneLoad: LoadStatus;
  phoneConnected: boolean;
  cloudLoad: LoadStatus;
  cloudConnected: boolean;

  /** Load a source once. No-ops while loading/ready (local also re-loads when
   *  `version` changes); errors are terminal until an invalidate/reload. */
  ensureLocal: (version: number) => void;
  ensurePhone: () => void;
  ensureCloud: () => void;
  /** Re-check local availability (e.g. on window focus) and reload only if a
   *  drive was plugged in or ejected since the last load. */
  revalidateLocal: () => void;
  /** Mark a source stale so the next `ensure*` reloads it (after pair/connect). */
  invalidatePhone: () => void;
  invalidateCloud: () => void;
  /** Force every source to reload on the next `ensure*` (e.g. a manual refresh). */
  reloadAll: () => void;
}

// Per-source generation tokens: bumped on (re)load and invalidation so a slow
// in-flight fetch that resolves after the source was invalidated is discarded.
let localGen = 0;
let phoneGen = 0;
let cloudGen = 0;
// Guards against overlapping focus probes (one cheap availability check at a time).
let revalidating = false;

/**
 * The single source of truth for the merged music library (local + phones +
 * cloud). Living in a store rather than component state is deliberate: the
 * Player view unmounts when you navigate away, so holding tracks here means a
 * source is fetched **once** and survives navigation — it only reloads when its
 * `ensure*` is told to (a library re-scan, a phone pairing, or a cloud
 * connect/disconnect), never just because you reopened the Library tab.
 */
export const useMusicLibraryStore = create<MusicLibraryStore>((set, get) => ({
  local: [],
  phone: [],
  cloudBase: [],
  cloudMeta: new Map(),
  localLoad: "idle",
  localVersion: -1,
  localTotal: 0,
  phoneLoad: "idle",
  phoneConnected: false,
  cloudLoad: "idle",
  cloudConnected: false,

  ensureLocal: (version) => {
    const s = get();
    if (s.localLoad === "loading") return;
    // Already loaded (or errored) for this exact library version → keep it.
    // A version bump (a re-scan in Settings) reloads, recovering from errors.
    if (s.localLoad !== "idle" && s.localVersion === version) return;
    const gen = ++localGen;
    // Start fresh: the library streams in page by page, never blocking the UI.
    set({ localLoad: "loading", localVersion: version, local: [], localTotal: 0 });
    loadLocalPaged({
      fetchCount: libraryCount,
      fetchPage: libraryListPage,
      publish: (tracks, total) => {
        if (gen !== localGen) return;
        set({ local: tracks, localTotal: total });
      },
      yieldToLoop: yieldToMain,
      isStale: () => gen !== localGen,
      now: () => performance.now(),
    })
      .then((completed) => {
        // The final publish already set `local`; only flip to ready on a real
        // finish (a cancelled load leaves status as-is for the next ensure).
        if (completed && gen === localGen) set({ localLoad: "ready" });
      })
      .catch(() => {
        if (gen !== localGen) return;
        set({ local: [], localLoad: "error" });
      });
  },

  ensurePhone: () => {
    const s = get();
    if (s.phoneLoad !== "idle") return;
    const gen = ++phoneGen;
    set({ phoneLoad: "loading" });
    linkPaired()
      .then(async (devices) => {
        const lists = await Promise.all(
          devices.map((d) =>
            linkLibrary(d.id)
              .then((tracks) =>
                tracks.map((t) => ({
                  ...phoneItem(d, t),
                  source: "phone" as const,
                  uid: `phone:${d.id}:${t.id}`,
                  genre: null,
                  // The real folder the track came from on the phone, so the
                  // Folders facet groups by it (falls back to the device name).
                  folder: t.folder ?? d.name,
                  artPath: null,
                  cover: null,
                })),
              )
              .catch(() => [] as MusicTrack[]),
          ),
        );
        if (gen !== phoneGen) return;
        const merged = lists.flat();
        set({ phone: merged, phoneConnected: devices.length > 0, phoneLoad: "ready" });
      })
      .catch(() => {
        if (gen !== phoneGen) return;
        set({ phone: [], phoneConnected: false, phoneLoad: "error" });
      });
  },

  ensureCloud: () => {
    const s = get();
    if (s.cloudLoad !== "idle") return;
    const gen = ++cloudGen;
    set({ cloudLoad: "loading", cloudMeta: new Map() });
    cloudStatus()
      .then(async (status) => {
        const connected = [
          status.googleConnected ? ("googleDrive" as const) : null,
          status.dropboxConnected ? ("dropbox" as const) : null,
        ].filter((p): p is CloudProvider => p !== null);
        if (gen !== cloudGen) return;
        if (connected.length === 0) {
          set({ cloudBase: [], cloudConnected: false, cloudLoad: "ready" });
          return;
        }

        // ---- Phase 1: instant. The cached listing + cached tags are pure disk
        // reads when warm (a relaunch), so the library appears immediately
        // rather than re-walking the account over the network as if just
        // connected. A cold cache (first connect) fetches once, here.
        const pages = await Promise.all(
          connected.map((provider) =>
            cloudAllAudio(provider, false)
              .then((page) => ({ provider, page }))
              .catch(() => ({
                provider,
                page: { entries: [], fromCache: false } as CloudAudioPage,
              })),
          ),
        );
        if (gen !== cloudGen) return;
        const base = pages.flatMap(({ provider, page }) =>
          entriesToTracks(provider, page.entries),
        );

        const cached = await Promise.all(
          connected.map((provider) =>
            cloudCachedMetadata(provider)
              .then((meta) => ({ provider, meta }))
              .catch(() => ({
                provider,
                meta: {} as Record<string, CloudTrackMeta>,
              })),
          ),
        );
        if (gen !== cloudGen) return;
        const meta = new Map<string, CloudTrackMeta>();
        for (const { provider, meta: m } of cached) {
          for (const [fileId, value] of Object.entries(m)) {
            meta.set(`cloud:${provider}:${fileId}`, value);
          }
        }
        set({ cloudBase: base, cloudMeta: meta, cloudConnected: true, cloudLoad: "ready" });

        // ---- Phase 2: background refresh. Re-list providers served from cache
        // so newly added/removed files surface, then resolve tags only for
        // tracks we don't already have cached (first sight). Everything below is
        // keyed on `gen`, so an invalidate/disconnect mid-flight discards it.
        const staleProviders = pages
          .filter(({ page }) => page.fromCache)
          .map(({ provider }) => provider);
        if (staleProviders.length > 0) {
          const fresh = await Promise.all(
            staleProviders.map((provider) =>
              cloudAllAudio(provider, true)
                .then((page) => ({ provider, entries: page.entries }))
                .catch(() => null),
            ),
          );
          if (gen !== cloudGen) return;
          const freshByProvider = new Map(
            fresh
              .filter(
                (f): f is { provider: CloudProvider; entries: CloudEntry[] } =>
                  f !== null,
              )
              .map((f) => [f.provider, f.entries]),
          );
          // Rebuild from fresh listings where we re-listed, else the phase-1 copy.
          const rebuilt = pages.flatMap(({ provider, page }) =>
            entriesToTracks(provider, freshByProvider.get(provider) ?? page.entries),
          );
          set({ cloudBase: rebuilt });
        }

        // Only tracks without cached metadata need a network read; the rest
        // already showed instantly from the cache in phase 1.
        const currentMeta = get().cloudMeta;
        const needMeta = get().cloudBase.filter((t) => !currentMeta.has(t.uid));
        if (needMeta.length === 0) return;
        await preloadCloudMeta(
          needMeta,
          (uid, resolved) => {
            if (gen !== cloudGen) return;
            const nextMeta = new Map(get().cloudMeta);
            nextMeta.set(uid, resolved);
            set({ cloudMeta: nextMeta });
          },
          () => gen !== cloudGen,
        );
      })
      .catch(() => {
        if (gen !== cloudGen) return;
        set({ cloudBase: [], cloudConnected: false, cloudLoad: "error" });
      });
  },

  revalidateLocal: () => {
    const s = get();
    // Only reconcile a settled library — a load in flight already reflects
    // current availability. One probe at a time.
    if (s.localLoad !== "ready" || revalidating) return;
    revalidating = true;
    const shownAtProbe = s.local.length;
    libraryAvailableCount()
      .then((available) => {
        // Still settled and unchanged since the probe, but the reachable count
        // differs from what's shown → a drive was plugged in or ejected →
        // reload. Flipping to `idle` makes the Library view's load effect
        // re-fetch (filtered); if it's unmounted, the reload happens lazily on
        // its next open.
        const now = get();
        if (
          now.localLoad === "ready" &&
          now.local.length === shownAtProbe &&
          available !== shownAtProbe
        ) {
          localGen++; // cancel any stragglers
          set({ localLoad: "idle" });
        }
      })
      .catch(() => {})
      .finally(() => {
        revalidating = false;
      });
  },

  invalidatePhone: () => {
    phoneGen++; // cancel any in-flight load
    set({ phoneLoad: "idle" });
  },

  invalidateCloud: () => {
    cloudGen++; // cancel any in-flight load + metadata preload
    set({ cloudLoad: "idle", cloudConnected: false, cloudBase: [], cloudMeta: new Map() });
  },

  reloadAll: () => {
    localGen++;
    phoneGen++;
    cloudGen++;
    set({ localLoad: "idle", phoneLoad: "idle", cloudLoad: "idle" });
  },
}));
