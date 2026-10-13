import { create } from "zustand";
import type { DeviceInfo } from "../protocol";
import type { RemoteView } from "./session";

/** What the player UI renders while the app is a remote: the active device's
 *  state (`view`, null in local mode) and the online players for the picker. */
interface RemoteStore {
  view: RemoteView | null;
  /** Online players (this computer included), for the device picker. */
  players: DeviceInfo[];
  /** The id of the player the hub considers active, if any. */
  activeId: string | null;
  selfId: string;
  setView(view: RemoteView | null): void;
  setPlayers(players: DeviceInfo[], activeId: string | null, selfId: string): void;
}

export const useRemoteStore = create<RemoteStore>((set) => ({
  view: null,
  players: [],
  activeId: null,
  selfId: "",
  setView: (view) => set({ view }),
  setPlayers: (players, activeId, selfId) => set({ players, activeId, selfId }),
}));
