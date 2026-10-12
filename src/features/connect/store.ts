import { create } from "zustand";
import type { ConnectStatus } from "@/lib/ipc";
import type { DeviceInfo } from "./protocol";

/** What the Hype Connect card shows: the hub connection and the account's
 *  linked devices (from REST on open, then live from `welcome`/`devices`). */
interface ConnectStore {
  status: ConnectStatus | null;
  devices: DeviceInfo[];
  activeDeviceId: string | null;
  setStatus: (status: ConnectStatus) => void;
  setDevices: (devices: DeviceInfo[], activeDeviceId: string | null) => void;
}

export const useConnectStore = create<ConnectStore>((set) => ({
  status: null,
  devices: [],
  activeDeviceId: null,
  setStatus: (status) => set({ status }),
  setDevices: (devices, activeDeviceId) => set({ devices, activeDeviceId }),
}));
