# src/features/player/SeekBar.tsx

- seekDisplayValue · function · L13-L20 — function seekDisplayValue( scrub: number | null, position: number, duration: number, ): number
- SeekBar · function · L34-L70 — function SeekBar({ position, duration, seekable, onSeek, className, }: { position: number; /** Track length in seconds; 0/unknown disables seeking. */ duration: number; /** Whether the active source can be scrubbed at all (streams flip to true * once their length is known). */ seekable: boolean; onSeek: (secs: number) => void; className?: string; })
