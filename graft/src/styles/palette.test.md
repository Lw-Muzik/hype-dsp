# src/styles/palette.test.ts

- Rgba · type · L16-L16 — type Rgba = { r: number; g: number; b: number; a: number };
- parseColor · function · L19-L34 — function parseColor(value: string): Rgba
- escapeRegExp · function · L37-L39 — function escapeRegExp(value: string): string
- tokens · function · L51-L62 — function tokens(selector: string): Record<string, string>
- theme · function · L65-L69 — function theme(name: "dark" | "light" | "dynamic"): Record<string, string>
- get · function · L71-L75 — function get(t: Record<string, string>, name: string): Rgba
- over · function · L78-L85 — function over(fg: Rgba, bg: Rgba): Rgba
- luminance · function · L87-L93 — function luminance({ r, g, b }: Rgba): number
- f · function · L88-L91 — f = (c: number)
- ratio · function · L95-L103 — function ratio(a: Rgba, b: Rgba): number
- tsxFiles · function · L178-L196 — function tsxFiles(dir: string): string[]
