import { lazy } from "react";
import type { ComponentType } from "react";
import type { SceneInfo } from "@/lib/ipc";
import { RadialSpectrum } from "./RadialSpectrum";
import { NeonBars } from "./NeonBars";
import { Oscilloscope } from "./Oscilloscope";
import { ParticleBurst } from "./ParticleBurst";
import { LiquidBlob } from "./LiquidBlob";

// 3D scenes are lazy so three.js only downloads when one is selected (they
// share a single deduped three chunk).
const AudioSphere = lazy(() =>
  import("./AudioSphere").then((m) => ({ default: m.AudioSphere })),
);
const ParticleGalaxy = lazy(() =>
  import("./ParticleGalaxy").then((m) => ({ default: m.ParticleGalaxy })),
);
const AudioTerrain = lazy(() =>
  import("./AudioTerrain").then((m) => ({ default: m.AudioTerrain })),
);
const Tunnel = lazy(() => import("./Tunnel").then((m) => ({ default: m.Tunnel })));
const EqCity = lazy(() => import("./EqCity").then((m) => ({ default: m.EqCity })));

/** Scene id → renderer (the backend lists the same ids). */
export const SCENE_COMPONENTS: Record<string, ComponentType> = {
  "radial-spectrum": RadialSpectrum,
  "neon-bars": NeonBars,
  oscilloscope: Oscilloscope,
  "particle-burst": ParticleBurst,
  "liquid-blob": LiquidBlob,
  "audio-sphere": AudioSphere,
  "particle-galaxy": ParticleGalaxy,
  "audio-terrain": AudioTerrain,
  tunnel: Tunnel,
  "eq-city": EqCity,
};

/** Fallback list when the backend registry isn't reachable (preview/offline). */
export const BUILT_SCENES: SceneInfo[] = [
  { id: "radial-spectrum", name: "Radial Spectrum", kind: "2d" },
  { id: "neon-bars", name: "Neon Bars", kind: "2d" },
  { id: "oscilloscope", name: "Oscilloscope", kind: "2d" },
  { id: "particle-burst", name: "Particle Burst", kind: "2d" },
  { id: "liquid-blob", name: "Liquid Blob", kind: "2d" },
  { id: "audio-sphere", name: "Audio Sphere", kind: "3d" },
  { id: "particle-galaxy", name: "Particle Galaxy", kind: "3d" },
  { id: "audio-terrain", name: "Audio Terrain", kind: "3d" },
  { id: "tunnel", name: "Tunnel", kind: "3d" },
  { id: "eq-city", name: "Equalizer City", kind: "3d" },
];
