# src/features/movies/MoviePlayer.tsx

- Status · type · L24-L24 — type Status = "loading" | "playing" | "paused" | "error";
- MoviePlayer · function · L41-L391 — function MoviePlayer({ title, poster, streams, initialQuality, onClose, }: { title: string; poster: string; streams: MovieStream[]; initialQuality: Quality; onClose: () => void; })
- onLoaded · function · L98-L110 — onLoaded = ()
- onLoadError · function · L111-L113 — onLoadError = ()
- onPlaying · function · L130-L130 — onPlaying = ()
- onWaiting · function · L131-L131 — onWaiting = ()
- onPause · function · L132-L132 — onPause = ()
- onError · function · L133-L133 — onError = ()
- onVolume · function · L134-L137 — onVolume = ()
- onTime · function · L138-L138 — onTime = ()
- onDuration · function · L139-L139 — onDuration = ()
- onProgress · function · L140-L143 — onProgress = ()
- changeVolume · function · L189-L194 — changeVolume = (value: number)
- seekTo · function · L195-L198 — seekTo = (t: number)
- onKey · function · L209-L234 — onKey = (e: KeyboardEvent)
