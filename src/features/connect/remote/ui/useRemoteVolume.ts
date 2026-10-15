import { useRef, useState } from "react";
import { sendRemote } from "../control";

const VOLUME_SEND_GAP_MS = 120;

/** Volume that follows the drag locally and reaches the device at most every
 *  120 ms, plus the final value on release. Shared by the remote bar and the
 *  top bar, which both drive the controlled device's volume. */
export function useRemoteVolume(volume: number) {
  const [drag, setDrag] = useState<number | null>(null);
  const lastSent = useRef(0);
  const send = (v: number) => {
    lastSent.current = Date.now();
    void sendRemote({ type: "volume", value: v });
  };
  return {
    value: drag ?? volume,
    onChange: (v: number) => {
      setDrag(v);
      if (Date.now() - lastSent.current >= VOLUME_SEND_GAP_MS) send(v);
    },
    onCommit: (v: number) => {
      send(v);
      setDrag(null);
    },
  };
}
