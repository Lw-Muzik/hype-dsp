/**
 * Playback position between the engine's progress events (~10/s): the last
 * reported position, advanced by the time since it was reported while playing.
 */
export class PositionExtrapolator {
  private posMs = 0;
  private at = 0;
  private running = false;

  /** The engine reported `posMs` at local time `atMs`. `running`: the position
   *  is advancing (playing, not paused, not buffering). */
  sync(posMs: number, atMs: number, running: boolean): void {
    this.posMs = Number.isFinite(posMs) ? Math.max(0, posMs) : 0;
    this.at = atMs;
    this.running = running;
  }

  positionAt(nowMs: number): number {
    return this.running ? this.posMs + Math.max(0, nowMs - this.at) : this.posMs;
  }
}
