# src/stores/ui.ts

- Route · type · L6-L16 — type Route = | "enhancer" | "equalizer" | "explore" | "mixer" | "movies" | "stems" | "player" | "stations" | "visuals" | "settings";
- RightPanel · type · L21-L21 — type RightPanel = "queue" | "lyrics" | "video";
- Side · type · L29-L29 — type Side = "left" | "right";
- clampWidth · function · L36-L39 — clampWidth = (w: number, side: Side): number
- loadWidth · function · L41-L49 — loadWidth = (side: Side): number
- saveWidth · function · L51-L57 — saveWidth = (side: Side, w: number): void
- loadEqBandCount · function · L62-L69 — loadEqBandCount = (): number
- saveEqBandCount · function · L71-L77 — saveEqBandCount = (n: number): void
- UiState · interface · L79-L112 — interface UiState
