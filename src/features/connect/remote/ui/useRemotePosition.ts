import { useEffect, useState } from "react";
import { remoteHubNow } from "../control";
import { positionNow, type RemoteView } from "../session";

const TICK_MS = 250;

/** The remote device's playback position in ms, ticking while it plays
 *  (`positionMs + (playing ? hubNow - at : 0)`, §3.6). */
export function useRemotePosition(view: RemoteView): number {
  const [pos, setPos] = useState(() => positionNow(view, remoteHubNow()));
  useEffect(() => {
    const read = () => setPos(positionNow(view, remoteHubNow()));
    read();
    if (!view.playing) return;
    const timer = window.setInterval(read, TICK_MS);
    return () => window.clearInterval(timer);
  }, [view]);
  return pos;
}
