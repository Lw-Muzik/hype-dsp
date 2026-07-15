import { create } from "zustand";
import {
  ipcErrorMessage,
  onYtDownload,
  ytmusicDownload,
  ytmusicDownloadToPhone,
} from "@/lib/ipc";
import { useLibraryStore } from "@/stores/library";
import { toast } from "@/stores/toast";
import type { PhoneDevice, YtDownloadProgress } from "@/lib/types";
import type { UnlistenFn } from "@tauri-apps/api/event";

/** An in-flight download's live progress. */
export interface DownloadState {
  /** "fetching" (YouTube → here) or "sending" (here → phone). */
  phase: YtDownloadProgress["phase"];
  bytes: number;
  /** Null until the transfer's length is known. */
  total: number | null;
}

/**
 * In-flight YouTube Music downloads, keyed by `videoId`.
 *
 * This lives in a store rather than the track row because a download takes
 * minutes and the library list is virtualized — the row that started it unmounts
 * as soon as you scroll away, and must find the progress still here when it
 * scrolls back. Completion is driven by the command's promise (which is
 * authoritative); the `ytmusic:download` event only feeds the progress bar.
 */
interface YtDownloadStore {
  active: Record<string, DownloadState>;
  /** Download to this machine; the backend also indexes it into the library. */
  toThisComputer: (videoId: string, title: string) => Promise<void>;
  /** Download here, then send it on to a paired phone. */
  toPhone: (videoId: string, title: string, device: PhoneDevice) => Promise<void>;
}

// One listener for the whole session, attached on the first download rather than
// at import: until then there is nothing to report, and this way no wiring is
// needed in App for a feature most launches never touch.
let listening: Promise<UnlistenFn> | null = null;

function ensureListening(): Promise<UnlistenFn> {
  listening ??= onYtDownload((p) => {
    useYtDownloadStore.setState((s) =>
      // Ignore progress for anything we're not tracking (e.g. a bare
      // `link_upload`, which keys by path), and let the promise — not the
      // event — decide when a download is finished.
      s.active[p.videoId] && (p.phase === "fetching" || p.phase === "sending")
        ? {
            active: {
              ...s.active,
              [p.videoId]: { phase: p.phase, bytes: p.bytes, total: p.total },
            },
          }
        : {},
    );
  });
  return listening;
}

export const useYtDownloadStore = create<YtDownloadStore>((set, get) => {
  /** Track `videoId` as started, or refuse when it already is. */
  const begin = async (videoId: string): Promise<boolean> => {
    if (get().active[videoId]) return false;
    // Subscribe *before* the command runs, or the first progress events race
    // the listener being attached and the bar starts part-way through.
    await ensureListening().catch(() => {});
    set((s) => ({
      active: { ...s.active, [videoId]: { phase: "fetching", bytes: 0, total: null } },
    }));
    return true;
  };

  const end = (videoId: string) =>
    set((s) => {
      const { [videoId]: _done, ...rest } = s.active;
      return { active: rest };
    });

  return {
    active: {},

    toThisComputer: async (videoId, title) => {
      if (!(await begin(videoId))) return;
      try {
        await ytmusicDownload(videoId);
        // The backend indexed it — bump the library so it shows up as a local
        // track (playable offline, seekable) without a manual re-scan.
        useLibraryStore.getState().refresh();
        toast.success(`Downloaded “${title}”.`);
      } catch (e) {
        toast.error(`Couldn't download “${title}”: ${ipcErrorMessage(e)}`);
      } finally {
        end(videoId);
      }
    },

    toPhone: async (videoId, title, device) => {
      if (!(await begin(videoId))) return;
      try {
        await ytmusicDownloadToPhone(videoId, device.id);
        // The laptop keeps its copy and indexes it, same as a plain download.
        useLibraryStore.getState().refresh();
        toast.success(`Sent “${title}” to ${device.name}.`);
      } catch (e) {
        toast.error(
          `Couldn't send “${title}” to ${device.name}: ${ipcErrorMessage(e)}`,
        );
      } finally {
        end(videoId);
      }
    },
  };
});
