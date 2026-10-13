# src/stores/radio.ts

- RadioSession · interface · L13-L16 — interface RadioSession
- RadioStep · type · L21-L25 — type RadioStep = | { kind: "continue"; seedId: string; token: string } | { kind: "reseed"; seedId: string } | { kind: "start"; seedId: string } | null;
- radioStep · function · L28-L55 — function radioStep(args: { autoplay: boolean; fetching: boolean; session: RadioSession | null; orderLen: number; orderPos: number; /** Whole queue is YT Music tracks — radio can only grow those. */ allYtMusic: boolean; /** videoId of the last track in play order: the seed for extension. */ lastVideoId: string | null; repeat: RepeatMode; }): RadioStep
- dedupeRadioTracks · function · L59-L68 — function dedupeRadioTracks(queue: QueueItem[], incoming: YtTrack[]): YtTrack[]
