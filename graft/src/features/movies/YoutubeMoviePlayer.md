# src/features/movies/YoutubeMoviePlayer.tsx

- Status · type · L8-L8 — type Status = "loading" | "ready" | "error";
- YtPlayerInstance · interface · L15-L17 — interface YtPlayerInstance
- YtPlayerCtor · interface · L18-L33 — interface YtPlayerCtor
- Window · interface · L35-L38 — interface Window
- loadIframeApi · function · L53-L69 — function loadIframeApi(): Promise<void>
- YoutubeMoviePlayer · function · L96-L250 — function YoutubeMoviePlayer({ title, videoId, onClose, }: { title: string; videoId: string; onClose: () => void; })
- onKey · function · L177-L179 — onKey = (e: KeyboardEvent)
