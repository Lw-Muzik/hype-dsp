# src/features/settings/systemAudioCard.ts

- SystemAudioAffordance · type · L4-L8 — type SystemAudioAffordance = | "enable" // ready now → Enable / Restart / Stop | "install" // driver bundled but not installed → "Install audio driver" | "not-bundled" // no signed driver in this build → one-click VB-CABLE setup | "unavailable";
- systemAudioAffordance · function · L18-L25 — function systemAudioAffordance( status: SystemAudioStatus, ): SystemAudioAffordance
