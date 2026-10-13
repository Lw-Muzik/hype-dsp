# src/features/visuals/VisualsView.tsx

- Tab · type · L27-L27 — type Tab = "scenes" | "milkdrop";
- pickPreset · function · L31-L41 — function pickPreset( all: string[], favorites: string[], current: string | null, ): string | null
- VisualsView · function · L49-L90 — function VisualsView()
- switchTab · function · L61-L68 — switchTab = (t: Tab)
- TabButton · function · L92-L114 — function TabButton({ active, onClick, children, }: { active: boolean; onClick: () => void; children: ReactNode; })
- ScenesPanel · function · L118-L229 — function ScenesPanel()
- select · function · L132-L136 — select = (id: string)
- setStageFullscreen · function · L144-L155 — setStageFullscreen = (on: boolean)
- onKey · function · L159-L161 — onKey = (e: KeyboardEvent)
- StageMessage · function · L231-L240 — function StageMessage({ children }: { children: ReactNode })
- MilkDropPanel · function · L244-L405 — function MilkDropPanel()
- match · function · L294-L294 — match = (n: string)
- PresetRow · function · L407-L454 — function PresetRow({ name, active, favorite, onSelect, onToggleFavorite, }: { name: string; active: boolean; favorite: boolean; onSelect: () => void; onToggleFavorite: () => void; })
