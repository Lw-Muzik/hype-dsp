# src/stores/networkMode.ts

- NetworkMode · type · L1-L1 — type NetworkMode = "unknown" | "fast" | "constrained";
- NetworkState · interface · L34-L40 — interface NetworkState
- observe · function · L45-L60 — function observe( prev: NetworkState, sample: { downloadBps: number; rebufferDelta: number }, now: number, ): NetworkState
- effectiveMode · function · L66-L69 — function effectiveMode(state: NetworkState, now: number): NetworkMode
- chooseStreamMode · function · L80-L88 — function chooseStreamMode( source: "cloud" | "phone" | "ytmusic", dataSaver: boolean, net: NetworkMode, ): "gapless" | "progressive"
- loadNetworkMode · function · L99-L113 — function loadNetworkMode(): NetworkState
- saveNetworkMode · function · L117-L124 — function saveNetworkMode(state: NetworkState): void
