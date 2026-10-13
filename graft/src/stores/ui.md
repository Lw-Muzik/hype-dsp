# src/stores/ui.ts

- Route · type · L6-L15 — type Route = | "enhancer" | "equalizer" | "explore" | "mixer" | "movies" | "stems" | "player" | "stations" | "visuals";
- RightPanel · type · L20-L20 — type RightPanel = "queue" | "lyrics" | "video";
- Side · type · L28-L28 — type Side = "left" | "right";
- clampWidth · function · L35-L38 — clampWidth = (w: number, side: Side): number
- loadWidth · function · L40-L48 — loadWidth = (side: Side): number
- saveWidth · function · L50-L56 — saveWidth = (side: Side, w: number): void
- loadEqBandCount · function · L61-L68 — loadEqBandCount = (): number
- saveEqBandCount · function · L70-L76 — saveEqBandCount = (n: number): void
- UiState · interface · L78-L111 — interface UiState
