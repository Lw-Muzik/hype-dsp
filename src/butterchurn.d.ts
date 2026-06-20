// Minimal ambient types for butterchurn + butterchurn-presets (both ship no
// types). Only the surface we use is declared.

declare module "butterchurn" {
  /** Time-domain byte data per channel (0–255, centered at 128). */
  export interface ButterchurnAudioLevels {
    timeByteArray: Uint8Array;
    timeByteArrayL: Uint8Array;
    timeByteArrayR: Uint8Array;
  }

  export interface ButterchurnVisualizer {
    loadPreset(preset: unknown, blendTime?: number): void;
    setRendererSize(width: number, height: number): void;
    render(opts?: { audioLevels?: ButterchurnAudioLevels; elapsedTime?: number }): void;
    connectAudio(node: AudioNode): void;
    launchSongTitleAnim?(title: string): void;
  }

  export interface ButterchurnCreateOpts {
    width: number;
    height: number;
    pixelRatio?: number;
    textureRatio?: number;
  }

  export interface Butterchurn {
    createVisualizer(
      audioContext: AudioContext,
      canvas: HTMLCanvasElement,
      opts: ButterchurnCreateOpts,
    ): ButterchurnVisualizer;
  }

  const butterchurn: Butterchurn;
  export default butterchurn;
}

declare module "butterchurn-presets" {
  export function getPresets(): Record<string, unknown>;
  const presets: { getPresets(): Record<string, unknown> };
  export default presets;
}
