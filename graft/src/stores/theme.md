# src/stores/theme.ts

- ThemeChoice · type · L3-L3 — type ThemeChoice = "system" | "dynamic" | "light" | "dark";
- ResolvedTheme · type · L5-L5 — type ResolvedTheme = "dynamic" | "light" | "dark";
- resolveTheme · function · L23-L26 — function resolveTheme(choice: ThemeChoice, prefersDark: boolean): ResolvedTheme
- parseChoice · function · L28-L30 — function parseChoice(raw: unknown): ThemeChoice
- clampBlur · function · L32-L36 — function clampBlur(raw: unknown): number
- loadChoice · function · L38-L44 — function loadChoice(): ThemeChoice
- loadBlur · function · L46-L53 — function loadBlur(): number
- save · function · L55-L61 — function save(key: string, value: string): void
- prefersDarkNow · function · L64-L70 — function prefersDarkNow(): boolean
- ThemeStore · interface · L72-L80 — interface ThemeStore
- watchPrefersDark · function · L103-L112 — function watchPrefersDark(onChange: (prefersDark: boolean) => void): () => void
- handler · function · L106-L106 — handler = (e: MediaQueryListEvent)
