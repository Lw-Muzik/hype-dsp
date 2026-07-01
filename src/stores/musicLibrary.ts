import { create } from "zustand";
import {
  cloudAllAudio,
  cloudCachedTags,
  cloudStatus,
  cloudTrackTags,
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
  CloudTrackMeta,
  LibraryPage,
  LibraryTrack,
  PhoneDevice,
  PhoneTrack,
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

/** Map one cloud audio entry (account-wide, all folders — mirrors the mobile
 *  app, so songs nested in subfolders are included) into a browsable
 *  `MusicTrack`. Cheap (O(1)); the listing is fetched + cached in `ensureCloud`,
 *  then mapped incrementally so a large account never blocks the UI. The uid is
 *  keyed by the owning account (not just the provider), so files from two
 *  accounts of the same provider never collide. */
function cloudEntryToTrack(e: CloudEntry): MusicTrack {
  return {
    ...cloudItem(e),
    source: "cloud" as const,
    uid: `cloud:${e.accountId}:${e.id}`,
    genre: null,
    folder: e.folder ?? null,
    artPath: null,
    cover: null,
  };
}

/** Map one paired-phone track into a browsable `MusicTrack` (O(1)). */
function phoneTrackToTrack(d: PhoneDevice, t: PhoneTrack): MusicTrack {
  return {
    ...phoneItem(d, t),
    source: "phone" as const,
    uid: `phone:${d.id}:${t.id}`,
    genre: null,
    // The real folder the track came from on the phone, so the Folders facet
    // groups by it (falls back to the device name).
    folder: t.folder ?? d.name,
    artPath: null,
    cover: null,
  };
}

// Chunk size for incrementally mapping a source that arrives as one in-memory
// array (phone/cloud). Mapping a track is a cheap object spread, so ~1000 per
// chunk stays well under a frame; we yield to the event loop between chunks.
const MAP_CHUNK_SIZE = 1000;

/** Injectable dependencies for {@link mapIncrementally} (pure + unit-testable). */
export interface IncrementalMapDeps<S, T> {
  /** The full source array (already fetched). */
  source: S[];
  /** Cheap O(1) per-item transform. */
  map: (item: S) => T;
  /** Publish the growing result (a fresh array) to the store. */
  publish: (mapped: T[]) => void;
  /** Yield to the event loop between chunks. */
  yieldToLoop: () => Promise<void>;
  /** True once this run was superseded (invalidate/reload) → stop now. */
  isStale: () => boolean;
  /** Monotonic clock (ms) for throttling publishes; injectable for tests. */
  now: () => number;
  chunkSize?: number;
  publishIntervalMs?: number;
}

/**
 * Map a large in-memory array into `MusicTrack`s without blocking the main
 * thread: process it in chunks, yield to the event loop between chunks, and
 * publish the growing result on a time budget (not per chunk) so the O(n)
 * consumer derivations run a few times a second instead of once per chunk.
 *
 * This is the single-array analogue of {@link loadLocalPaged} (which pages from
 * the DB): phone and cloud libraries arrive as one array, so they map this way.
 * Returns the complete mapped array, or `null` if cancelled mid-run.
 */
export async function mapIncrementally<S, T>(
  deps: IncrementalMapDeps<S, T>,
): Promise<T[] | null> {
  const {
    source,
    map,
    publish,
    yieldToLoop,
    isStale,
    now,
    chunkSize = MAP_CHUNK_SIZE,
    publishIntervalMs = LOCAL_PUBLISH_INTERVAL_MS,
  } = deps;

  const acc: T[] = [];
  let lastPublish = -Infinity;
  for (let i = 0; i < source.length; i++) {
    acc.push(map(source[i]!));
    if ((i + 1) % chunkSize === 0) {
      // Throttle publishes (cheap to skip), but always yield so input/paint
      // get a turn between chunks even when we don't publish this one.
      if (now() - lastPublish >= publishIntervalMs) {
        publish(acc.slice());
        lastPublish = now();
      }
      await yieldToLoop();
      if (isStale()) return null;
    }
  }
  // Final publish of the complete set (also covers an empty source).
  publish(acc.slice());
  return acc;
}

// How many cloud files to read metadata for at once (the backend caches each
// after the first read, so this only bites on the first scan), and how often to
// flush resolved tags to the store. Flushing on a time budget — rather than per
// resolved track — keeps the metadata Map rebuild (O(n)) and the re-derivation
// it triggers down to a few per second instead of one per track (which made a
// full scan O(n²)).
const CLOUD_META_CONCURRENCY = 4;
const CLOUD_META_FLUSH_MS = 200;

/**
 * Read embedded **text tags** (title/artist/album — no cover) for cloud tracks
 * in the background, like the mobile app's `CloudMetadataService`. Runs a few at
 * a time and hands resolved tags to `onBatch` in time-budgeted batches (so the
 * store rebuilds its metadata Map a few times a second, not once per track).
 * `isStale` lets the caller cancel (disconnect / reload). Backend-cached →
 * re-scans cheap. Covers are deliberately excluded here and resolved lazily per
 * on-screen row instead, so a large library never holds thousands of ~100 KB
 * base64 covers in memory (the old behaviour that spiked memory + jank on load).
 */
async function preloadCloudMeta(
  tracks: MusicTrack[],
  onBatch: (updates: Array<[string, CloudTrackMeta]>) => void,
  isStale: () => boolean,
  now: () => number,
): Promise<void> {
  // Workers share one JS thread cooperatively (await points), so mutating this
  // buffer between awaits is race-free.
  const pending: Array<[string, CloudTrackMeta]> = [];
  let lastFlush = -Infinity;
  const flush = (force: boolean) => {
    if (pending.length === 0) return;
    if (!force && now() - lastFlush < CLOUD_META_FLUSH_MS) return;
    onBatch(pending.splice(0)); // hand off everything pending and clear
    lastFlush = now();
  };

  let next = 0;
  const worker = async () => {
    while (next < tracks.length && !isStale()) {
      const t = tracks[next++];
      const file = t?.cloud;
      if (!t || !file) continue;
      try {
        const meta = await cloudTrackTags(file.accountId, file.id, file.name);
        if (meta && !isStale()) {
          pending.push([t.uid, meta]);
          flush(false);
        }
      } catch {
        // Skip — a single failed read shouldn't stop the rest.
      }
    }
  };
  await Promise.all(
    Array.from({ length: CLOUD_META_CONCURRENCY }, () => worker()),
  );
  flush(true); // trailing flush of the remainder
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
  /** Re-check paired phones and refresh their libraries **in place** (keeping the
   *  current tracks visible, no empty flash). Runs regardless of whether the
   *  Library view is mounted, so a phone coming online auto-syncs without a
   *  relaunch. */
  syncPhone: () => void;
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
    const isStale = () => gen !== phoneGen;
    set({ phoneLoad: "loading", phone: [] });
    (async () => {
      const devices = await linkPaired();
      if (isStale()) return;
      // Reflect connectivity immediately so the source pill updates while the
      // (possibly large) per-device libraries stream in below.
      set({ phoneConnected: devices.length > 0 });
      // Each device's library is fetched off the main thread; a failure on one
      // contributes nothing rather than breaking the whole phone source.
      const libs = await Promise.all(
        devices.map((d) =>
          linkLibrary(d.id)
            .then((tracks) => ({ d, tracks }))
            .catch(() => ({ d, tracks: [] as PhoneTrack[] })),
        ),
      );
      if (isStale()) return;
      // Flatten to (device, track) pairs, then map incrementally so a phone
      // with thousands of songs populates progressively instead of freezing.
      const pairs = libs.flatMap(({ d, tracks }) =>
        tracks.map((t) => ({ d, t })),
      );
      const ok = await mapIncrementally({
        source: pairs,
        map: ({ d, t }) => phoneTrackToTrack(d, t),
        publish: (mapped) => {
          if (!isStale()) set({ phone: mapped });
        },
        yieldToLoop: yieldToMain,
        isStale,
        now: () => performance.now(),
      });
      if (ok && !isStale()) set({ phoneLoad: "ready" });
    })().catch(() => {
      if (isStale()) return;
      set({ phone: [], phoneConnected: false, phoneLoad: "error" });
    });
  },

  syncPhone: () => {
    // Supersede any in-flight phone load/sync (its writes are discarded).
    const gen = ++phoneGen;
    const isStale = () => gen !== phoneGen;
    // Mark loading but KEEP the current tracks on screen — this is a refresh,
    // not a cold load, so the new set swaps in only once it's ready.
    set({ phoneLoad: "loading" });
    (async () => {
      const devices = await linkPaired();
      if (isStale()) return;
      set({ phoneConnected: devices.length > 0 });
      const libs = await Promise.all(
        devices.map((d) =>
          linkLibrary(d.id)
            .then((tracks) => ({ d, tracks }))
            .catch(() => ({ d, tracks: [] as PhoneTrack[] })),
        ),
      );
      if (isStale()) return;
      const pairs = libs.flatMap(({ d, tracks }) =>
        tracks.map((t) => ({ d, t })),
      );
      // Map in chunks (yielding between them) but publish only the final set,
      // so a large phone library refreshes without blocking and without a
      // shrink-then-grow flash of the currently-shown tracks.
      const mapped = await mapIncrementally({
        source: pairs,
        map: ({ d, t }) => phoneTrackToTrack(d, t),
        publish: () => {},
        yieldToLoop: yieldToMain,
        isStale,
        now: () => performance.now(),
      });
      if (mapped && !isStale()) set({ phone: mapped, phoneLoad: "ready" });
    })().catch(() => {
      // Leave the existing tracks in place on failure; just settle the status.
      if (!isStale()) set({ phoneLoad: "ready" });
    });
  },

  ensureCloud: () => {
    const s = get();
    if (s.cloudLoad !== "idle") return;
    const gen = ++cloudGen;
    const isStale = () => gen !== cloudGen;
    const now = () => performance.now();
    set({ cloudLoad: "loading", cloudMeta: new Map() });
    (async () => {
      const status = await cloudStatus();
      if (isStale()) return;
      const accounts = status.accounts;
      if (accounts.length === 0) {
        set({ cloudBase: [], cloudConnected: false, cloudLoad: "ready" });
        return;
      }

      // ---- Phase 1: instant. The cached listing + cached tags are pure disk
      // reads when warm (a relaunch), so the library appears immediately rather
      // than re-walking each account over the network as if just connected. A
      // cold cache (first connect) fetches once, here. Every connected account
      // (any number per provider) is listed and merged into one "Cloud" source.
      const pages = await Promise.all(
        accounts.map((acc) =>
          cloudAllAudio(acc.id, false)
            .then((page) => ({ acc, page }))
            .catch(() => ({
              acc,
              page: { entries: [], fromCache: false } as CloudAudioPage,
            })),
        ),
      );
      if (isStale()) return;
      // Reflect connectivity immediately so the source pill updates while the
      // (possibly large) listings map in below.
      set({ cloudConnected: true });

      // Map every account's flat listing into tracks without blocking — big
      // accounts populate progressively instead of freezing. Each entry already
      // carries its `accountId` (stamped by the backend), so the uid is unique.
      const baseEntries = pages.flatMap(({ page }) => page.entries);
      await mapIncrementally({
        source: baseEntries,
        map: cloudEntryToTrack,
        publish: (mapped) => {
          if (!isStale()) set({ cloudBase: mapped });
        },
        yieldToLoop: yieldToMain,
        isStale,
        now,
      });
      if (isStale()) return;

      // Merge any cached tags over the listed filenames (a single cheap pass),
      // keyed per account so two accounts' files never share a meta entry.
      // Tags only — covers are resolved lazily per visible row, so hydrating a
      // huge library on launch stays light instead of loading every cover.
      const cached = await Promise.all(
        accounts.map((acc) =>
          cloudCachedTags(acc.id)
            .then((meta) => ({ acc, meta }))
            .catch(() => ({
              acc,
              meta: {} as Record<string, CloudTrackMeta>,
            })),
        ),
      );
      if (isStale()) return;
      const meta = new Map<string, CloudTrackMeta>();
      for (const { acc, meta: m } of cached) {
        for (const [fileId, value] of Object.entries(m)) {
          meta.set(`cloud:${acc.id}:${fileId}`, value);
        }
      }
      set({ cloudMeta: meta, cloudLoad: "ready" });

      // ---- Phase 2: background refresh. Re-list accounts served from cache so
      // newly added/removed files surface, then resolve tags only for tracks we
      // don't already have cached (first sight). Everything below is keyed on
      // `gen`, so an invalidate/disconnect mid-flight discards it.
      const staleAccounts = pages
        .filter(({ page }) => page.fromCache)
        .map(({ acc }) => acc);
      if (staleAccounts.length > 0) {
        const fresh = await Promise.all(
          staleAccounts.map((acc) =>
            cloudAllAudio(acc.id, true)
              .then((page) => ({ accountId: acc.id, entries: page.entries }))
              .catch(() => null),
          ),
        );
        if (isStale()) return;
        const freshByAccount = new Map(
          fresh
            .filter(
              (f): f is { accountId: string; entries: CloudEntry[] } =>
                f !== null,
            )
            .map((f) => [f.accountId, f.entries]),
        );
        // Rebuild from fresh listings where we re-listed, else the phase-1 copy
        // — incrementally, so a large refresh stays non-blocking too.
        const rebuilt = pages.flatMap(
          ({ acc, page }) => freshByAccount.get(acc.id) ?? page.entries,
        );
        await mapIncrementally({
          source: rebuilt,
          map: cloudEntryToTrack,
          publish: (mapped) => {
            if (!isStale()) set({ cloudBase: mapped });
          },
          yieldToLoop: yieldToMain,
          isStale,
          now,
        });
        if (isStale()) return;
      }

      // Only tracks without cached metadata need a network read; the rest
      // already showed instantly from the cache in phase 1.
      const currentMeta = get().cloudMeta;
      const needMeta = get().cloudBase.filter((t) => !currentMeta.has(t.uid));
      if (needMeta.length === 0) return;
      await preloadCloudMeta(
        needMeta,
        (updates) => {
          if (isStale()) return;
          const nextMeta = new Map(get().cloudMeta);
          for (const [uid, resolved] of updates) nextMeta.set(uid, resolved);
          set({ cloudMeta: nextMeta });
        },
        isStale,
        now,
      );
    })().catch(() => {
      if (isStale()) return;
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
