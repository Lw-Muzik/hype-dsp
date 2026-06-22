import { create } from "zustand";
import {
  cloudAllAudio,
  cloudCachedMetadata,
  cloudStatus,
  cloudTrackMetadata,
  libraryList,
  linkLibrary,
  linkPaired,
} from "@/lib/ipc";
import { cloudItem, localItem, phoneItem } from "@/stores/engine";
import type { QueueItem } from "@/stores/engine";
import type { CloudAudioPage, CloudEntry, CloudProvider, CloudTrackMeta } from "@/lib/types";

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
  count: number;
}

/** The immediate parent folder name of a file path (for the Folders facet). */
function parentFolder(path: string): string | null {
  const norm = path.replace(/\\/g, "/").replace(/\/+$/, "");
  const parts = norm.split("/").filter(Boolean);
  return parts.length >= 2 ? parts[parts.length - 2]! : null;
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
  phoneLoad: LoadStatus;
  phoneConnected: boolean;
  cloudLoad: LoadStatus;
  cloudConnected: boolean;

  /** Load a source once. No-ops while loading/ready (local also re-loads when
   *  `version` changes); errors are terminal until an invalidate/reload. */
  ensureLocal: (version: number) => void;
  ensurePhone: () => void;
  ensureCloud: () => void;
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
    set({ localLoad: "loading", localVersion: version });
    libraryList()
      .then((tracks) => {
        if (gen !== localGen) return;
        set({
          local: tracks.map((t) => ({
            ...localItem(t),
            source: "local" as const,
            uid: `local:${t.path}`,
            genre: t.genre,
            folder: parentFolder(t.path),
            artPath: t.path,
            cover: null,
          })),
          localLoad: "ready",
        });
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
