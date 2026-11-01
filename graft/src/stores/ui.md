# src/stores/ui.ts

- Route · type · L6-L14 — type Route = | "enhancer" | "equalizer" | "explore" | "mixer" | "stems" | "player" | "stations" | "visuals";
- RightPanel · type · L19-L19 — type RightPanel = "queue" | "lyrics" | "video";
- Side · type · L27-L27 — type Side = "left" | "right";
- clampWidth · function · L34-L37 — clampWidth = (w: number, side: Side): number
- loadWidth · function · L39-L47 — loadWidth = (side: Side): number
- saveWidth · function · L49-L55 — saveWidth = (side: Side, w: number): void
- loadEqBandCount · function · L60-L67 — loadEqBandCount = (): number
- saveEqBandCount · function · L69-L75 — saveEqBandCount = (n: number): void
- loadRoute · function · L99-L106 — loadRoute = (): Route
- loadRightPanel · function · L109-L116 — loadRightPanel = (): RightPanel | null
- loadSidebarCollapsed · function · L118-L124 — loadSidebarCollapsed = (): boolean
- persist · function · L126-L133 — persist = (key: string, value: string | null): void
- UiState · interface · L135-L168 — interface UiState
