import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  CircleAlert,
  Globe,
  Play,
  QrCode,
  RefreshCw,
  Smartphone,
  Trash2,
  Wifi,
} from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import {
  ipcErrorMessage,
  linkArtwork,
  linkDiscover,
  linkDiscoverStart,
  linkDiscoverStop,
  linkLibrary,
  linkPair,
  linkPairAddress,
  linkPaired,
  linkRemoteCancel,
  linkRemoteConnect,
  linkRemoteForget,
  linkRemoteQr,
  linkRemoteStatus,
  linkUnpair,
  onPhoneFound,
  onRemoteConnected,
} from "@/lib/ipc";
import type { RemotePairingInfo, RemotePhoneStatus } from "@/lib/ipc";
import type { PhoneDevice, PhoneTrack } from "@/lib/types";
import { formatTime } from "@/lib/format";
import { coverGradient, coverInitials } from "@/lib/cover";
import { cn } from "@/lib/cn";

/** A small square gradient cover for a phone track. */
function Thumb({ seed, label }: { seed: string; label: string }) {
  return (
    <div
      className="grid size-11 shrink-0 place-items-center overflow-hidden rounded-md text-sm font-semibold text-white/90"
      style={{ background: coverGradient(seed) }}
      aria-hidden="true"
    >
      <span className="opacity-80">{coverInitials(label)}</span>
    </div>
  );
}

// Resolve+dedupe artwork lookups across renders (null = no art / unreachable).
const artCache = new Map<string, Promise<string | null>>();
function fetchArt(deviceId: string, trackId: string): Promise<string | null> {
  const key = `${deviceId}:${trackId}`;
  let pending = artCache.get(key);
  if (!pending) {
    pending = linkArtwork(deviceId, trackId).catch(() => null);
    artCache.set(key, pending);
  }
  return pending;
}

/** Real embedded artwork for a phone track, falling back to the gradient. */
function PhoneCover({ deviceId, track }: { deviceId: string; track: PhoneTrack }) {
  const [uri, setUri] = useState<string | null>(null);
  useEffect(() => {
    if (!track.hasArt) {
      setUri(null);
      return;
    }
    let active = true;
    void fetchArt(deviceId, track.id).then((u) => {
      if (active) setUri(u);
    });
    return () => {
      active = false;
    };
  }, [deviceId, track.id, track.hasArt]);

  if (uri) {
    return (
      <img
        src={uri}
        alt=""
        className="size-11 shrink-0 rounded-md object-cover"
        aria-hidden="true"
      />
    );
  }
  return <Thumb seed={track.album?.trim() || track.title} label={track.title} />;
}

/** Phone Link UI — discover phones on the LAN, pair via PIN, browse + play a
 *  phone's library. Lives as a section inside Settings (the connect flow); the
 *  Player only filters already-paired phones' songs into its unified list. */
export function DevicesView() {
  const nowPlaying = useEngineStore((s) => s.nowPlaying);
  const playPhoneList = useEngineStore((s) => s.playPhoneList);

  const [devices, setDevices] = useState<PhoneDevice[]>([]);
  const [pairedIds, setPairedIds] = useState<Set<string>>(new Set());
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The device whose library we're viewing.
  const [open, setOpen] = useState<PhoneDevice | null>(null);
  const [tracks, setTracks] = useState<PhoneTrack[]>([]);
  const [loading, setLoading] = useState(false);

  // PIN entry for an unpaired device.
  const [pairing, setPairing] = useState<PhoneDevice | null>(null);
  const [pin, setPin] = useState("");
  const [pinBusy, setPinBusy] = useState(false);

  // Manual "connect by address" (when discovery can't see the phone).
  const [manualAddr, setManualAddr] = useState("");
  const [manualPin, setManualPin] = useState("");
  const [manualBusy, setManualBusy] = useState(false);

  // Remote (cross-network, iroh) phones + the QR pairing session.
  const [remotePhones, setRemotePhones] = useState<RemotePhoneStatus[]>([]);
  const [pairingInfo, setPairingInfo] = useState<RemotePairingInfo | null>(null);
  const [remoteBusy, setRemoteBusy] = useState(false);

  /** Load paired phones + browse the LAN, merged (paired first). */
  const scan = useCallback(async () => {
    setScanning(true);
    setError(null);
    try {
      const paired = await linkPaired();
      const pairedSet = new Set(paired.map((d) => d.id));
      setPairedIds(pairedSet);
      let fresh = paired;
      try {
        const found = await linkDiscover();
        const extra = found.filter((d) => !pairedSet.has(d.id));
        fresh = [...paired, ...extra];
      } catch (e) {
        // Discovery can fail (no network); still show paired devices.
        setError(ipcErrorMessage(e));
      }
      // Merge by id so phones already surfaced by continuous discovery aren't
      // dropped when paired/one-shot results come in.
      setDevices((prev) => {
        const byId = new Map(prev.map((d) => [d.id, d]));
        for (const d of fresh) byId.set(d.id, d);
        return [...byId.values()];
      });
    } catch (e) {
      setError(ipcErrorMessage(e));
    } finally {
      setScanning(false);
    }
  }, []);

  useEffect(() => {
    void scan();
  }, [scan]);

  // Continuous discovery — phones appear the instant they're seen, no refresh.
  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;
    void linkDiscoverStart().catch(() => {});
    onPhoneFound((dev) => {
      setDevices((prev) => {
        const i = prev.findIndex((d) => d.id === dev.id);
        if (i < 0) return [...prev, dev];
        const cur = prev[i];
        if (!cur) return prev;
        const next = prev.slice();
        next[i] = { ...cur, name: dev.name, host: dev.host, port: dev.port };
        return next;
      });
    })
      .then((fn) => (cancelled ? fn() : (un = fn)))
      .catch(() => {});
    return () => {
      cancelled = true;
      un?.();
      void linkDiscoverStop().catch(() => {});
    };
  }, []);

  const browse = useCallback(async (device: PhoneDevice) => {
    setOpen(device);
    setLoading(true);
    setError(null);
    try {
      setTracks(await linkLibrary(device.id));
    } catch (e) {
      setError(ipcErrorMessage(e));
      setTracks([]);
    } finally {
      setLoading(false);
    }
  }, []);

  const onSelect = (device: PhoneDevice) => {
    if (pairedIds.has(device.id)) {
      void browse(device);
    } else {
      setPairing(device);
      setPin("");
      setError(null);
    }
  };

  const submitPin = async () => {
    if (!pairing || pin.length < 4) return;
    setPinBusy(true);
    setError(null);
    try {
      const device = await linkPair(
        pairing.host,
        pairing.port,
        pairing.name,
        pairing.id,
        pin,
      );
      setPairedIds((s) => new Set(s).add(device.id));
      setPairing(null);
      setPin("");
      await browse(device);
    } catch (e) {
      setError(ipcErrorMessage(e));
    } finally {
      setPinBusy(false);
    }
  };

  const connectByAddress = async () => {
    const addr = manualAddr.trim().replace(/^https?:\/\//, "");
    const m = addr.match(/^(.+):(\d{1,5})$/);
    if (!m) {
      setError("Enter the phone's address as host:port (e.g. 192.168.1.5:54321).");
      return;
    }
    const host = m[1]!;
    const port = Number(m[2]);
    if (manualPin.length < 4) {
      setError("Enter the pairing code shown on your phone.");
      return;
    }
    setManualBusy(true);
    setError(null);
    try {
      const device = await linkPairAddress(host, port, manualPin);
      setPairedIds((s) => new Set(s).add(device.id));
      setDevices((d) =>
        d.some((x) => x.id === device.id) ? d : [...d, device],
      );
      setManualAddr("");
      setManualPin("");
      await browse(device);
    } catch (e) {
      setError(ipcErrorMessage(e));
    } finally {
      setManualBusy(false);
    }
  };

  const unpair = async (device: PhoneDevice) => {
    await linkUnpair(device.id).catch(() => {});
    if (open?.id === device.id) {
      setOpen(null);
      setTracks([]);
    }
    await scan();
  };

  // --- remote (cross-network) phones ---

  const refreshRemote = useCallback(async () => {
    try {
      setRemotePhones(await linkRemoteStatus());
    } catch {
      /* manager may be unavailable — leave the list as-is */
    }
  }, []);

  const startRemotePairing = async () => {
    setRemoteBusy(true);
    setError(null);
    try {
      setPairingInfo(await linkRemoteQr());
    } catch (e) {
      setError(ipcErrorMessage(e));
    } finally {
      setRemoteBusy(false);
    }
  };

  const stopRemotePairing = async () => {
    setPairingInfo(null);
    await linkRemoteCancel().catch(() => {});
  };

  const forgetRemote = async (id: string) => {
    await linkRemoteForget(id).catch(() => {});
    if (open?.id === id) {
      setOpen(null);
      setTracks([]);
    }
    await refreshRemote();
    await scan();
  };

  // Redial known remote phones on open, and refresh whenever one pairs/connects.
  useEffect(() => {
    void linkRemoteConnect()
      .then(setRemotePhones)
      .catch(() => void refreshRemote());
    let un: (() => void) | undefined;
    let cancelled = false;
    onRemoteConnected(() => {
      setPairingInfo(null);
      void refreshRemote();
      void scan();
    })
      .then((fn) => (cancelled ? fn() : (un = fn)))
      .catch(() => {});
    return () => {
      cancelled = true;
      un?.();
    };
  }, [refreshRemote, scan]);

  const back = () => {
    setOpen(null);
    setTracks([]);
    setError(null);
  };

  const errorBanner = error && (
    <div className="flex items-start gap-2 rounded-control border border-danger/30 bg-danger/10 px-3 py-2 text-sm">
      <CircleAlert className="mt-0.5 size-4 shrink-0 text-danger" aria-hidden="true" />
      <span>{error}</span>
    </div>
  );

  // Remote phones are reachable over iroh (host 127.0.0.1 loopback), not on the
  // LAN — keep them out of the "Phones on your network" list; they get their own
  // panel below.
  const remoteIds = useMemo(
    () => new Set(remotePhones.map((p) => p.id)),
    [remotePhones],
  );
  const lanDevices = useMemo(
    () => devices.filter((d) => !remoteIds.has(d.id)),
    [devices, remoteIds],
  );

  const content = open ? (
    <DeviceLibrary
      device={open}
      tracks={tracks}
      loading={loading}
      nowPlaying={nowPlaying}
      onBack={back}
      onPlay={(t) =>
        playPhoneList(
          open,
          tracks,
          tracks.findIndex((x) => x.id === t.id),
        )
      }
      banner={errorBanner}
    />
  ) : (
        <div className="flex flex-col gap-4">
          <Card
            title="Phones on your network"
            icon={Smartphone}
            actions={
              <button
                type="button"
                aria-label="Rescan"
                onClick={() => void scan()}
                className="text-text-muted transition-colors hover:text-text"
              >
                <RefreshCw
                  className={cn("size-4", scanning && "animate-spin")}
                  aria-hidden="true"
                />
              </button>
            }
          >
            {pairing ? (
              <PinForm
                device={pairing}
                pin={pin}
                busy={pinBusy}
                onPin={setPin}
                onSubmit={() => void submitPin()}
                onCancel={() => setPairing(null)}
              />
            ) : lanDevices.length === 0 ? (
              <div className="flex flex-col items-center gap-2 py-6 text-center">
                <Wifi className="size-7 text-text-faint" aria-hidden="true" />
                <p className="text-sm text-text-muted">
                  {scanning
                    ? "Looking for phones…"
                    : "No phones found. Open Hype Muzik on your phone, enable “Stream / Cast”, and make sure both devices are on the same Wi‑Fi."}
                </p>
              </div>
            ) : (
              <ul className="divide-y divide-border">
                {lanDevices.map((d) => {
                  const isPaired = pairedIds.has(d.id);
                  return (
                    <li key={d.id} className="flex items-center gap-3 py-2.5">
                      <Smartphone
                        className="size-5 shrink-0 text-text-muted"
                        aria-hidden="true"
                      />
                      <button
                        type="button"
                        onClick={() => onSelect(d)}
                        className="flex min-w-0 flex-1 items-center gap-2 text-left"
                      >
                        <span className="min-w-0">
                          <span className="block truncate text-sm font-medium">
                            {d.name}
                          </span>
                          <span className="block truncate text-xs text-text-faint">
                            {d.host}
                          </span>
                        </span>
                      </button>
                      {isPaired ? (
                        <span className="rounded-control bg-success/15 px-2 py-0.5 text-xs text-success">
                          Paired
                        </span>
                      ) : (
                        <span className="text-xs text-text-faint">tap to pair</span>
                      )}
                      {isPaired && (
                        <button
                          type="button"
                          aria-label={`Unpair ${d.name}`}
                          onClick={() => void unpair(d)}
                          className="flex size-7 items-center justify-center rounded-control text-text-faint hover:bg-surface hover:text-danger"
                        >
                          <Trash2 className="size-4" aria-hidden="true" />
                        </button>
                      )}
                      <button
                        type="button"
                        aria-label={`Open ${d.name}`}
                        onClick={() => onSelect(d)}
                        className="text-text-faint transition-colors hover:text-text"
                      >
                        <ChevronRight className="size-4" aria-hidden="true" />
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
            {!pairing && (
              <div className="mt-4 border-t border-border pt-3">
                <p className="text-xs font-medium text-text-muted">
                  Or connect by address
                </p>
                <p className="mt-1 text-xs text-text-faint">
                  If your phone doesn’t appear above, open Stream / Cast on it and
                  enter the address it shows with the pairing code. Works across
                  networks too (e.g. over a VPN).
                </p>
                <div className="mt-2 flex flex-col gap-2 sm:flex-row">
                  <input
                    value={manualAddr}
                    onChange={(e) => setManualAddr(e.target.value)}
                    placeholder="192.168.1.5:54321"
                    aria-label="Phone address"
                    className="min-w-0 flex-1 rounded-control border border-border bg-surface px-3 py-2 text-sm outline-none placeholder:text-text-faint focus:border-accent/50"
                  />
                  <input
                    value={manualPin}
                    onChange={(e) =>
                      setManualPin(e.target.value.replace(/\D/g, "").slice(0, 8))
                    }
                    placeholder="Code"
                    inputMode="numeric"
                    aria-label="Pairing code"
                    className="w-full rounded-control border border-border bg-surface px-3 py-2 text-sm tabular-nums outline-none placeholder:text-text-faint focus:border-accent/50 sm:w-28"
                  />
                  <Button
                    variant="primary"
                    disabled={manualBusy}
                    onClick={() => void connectByAddress()}
                  >
                    {manualBusy ? "Connecting…" : "Connect"}
                  </Button>
                </div>
              </div>
            )}
            {!pairing && errorBanner && <div className="mt-3">{errorBanner}</div>}
          </Card>

          <Card
            title="Connect across networks"
            icon={Globe}
            actions={
              <button
                type="button"
                aria-label="Refresh remote phones"
                onClick={() => void refreshRemote()}
                className="text-text-muted transition-colors hover:text-text"
              >
                <RefreshCw className="size-4" aria-hidden="true" />
              </button>
            }
          >
            {pairingInfo ? (
              <div className="flex flex-col items-center gap-3 py-1 text-center">
                <p className="text-sm text-text-muted">
                  In Hype Muzik on your phone, open{" "}
                  <span className="font-medium text-text">Stream / Cast → Link a desktop</span>{" "}
                  and scan this code.
                </p>
                <div className="rounded-xl bg-white p-3">
                  <QRCodeSVG value={pairingInfo.qr} size={184} marginSize={1} />
                </div>
                <p className="text-xs text-text-faint">
                  Pairing code{" "}
                  <span className="font-mono tabular-nums text-text">
                    {pairingInfo.pin}
                  </span>
                </p>
                <Button variant="secondary" onClick={() => void stopRemotePairing()}>
                  Done
                </Button>
              </div>
            ) : (
              <>
                {remotePhones.length === 0 ? (
                  <p className="py-2 text-sm text-text-muted">
                    No phones linked across networks yet. Pair one to stream its
                    music over the internet — securely, peer-to-peer.
                  </p>
                ) : (
                  <ul className="divide-y divide-border">
                    {remotePhones.map((p) => (
                      <li key={p.id} className="flex items-center gap-3 py-2.5">
                        <Globe
                          className="size-5 shrink-0 text-text-muted"
                          aria-hidden="true"
                        />
                        <button
                          type="button"
                          disabled={!p.online || p.port == null}
                          onClick={() =>
                            p.port != null &&
                            void browse({
                              id: p.id,
                              name: p.name,
                              host: "127.0.0.1",
                              port: p.port,
                            })
                          }
                          className="flex min-w-0 flex-1 items-center gap-2 text-left disabled:cursor-default"
                        >
                          <span className="min-w-0">
                            <span className="block truncate text-sm font-medium">
                              {p.name}
                            </span>
                            <span className="block truncate text-xs text-text-faint">
                              {p.online ? "Connected" : "Offline"}
                            </span>
                          </span>
                        </button>
                        <span
                          className={cn(
                            "rounded-control px-2 py-0.5 text-xs",
                            p.online
                              ? "bg-success/15 text-success"
                              : "bg-surface text-text-faint",
                          )}
                        >
                          {p.online ? "Online" : "Offline"}
                        </span>
                        <button
                          type="button"
                          aria-label={`Forget ${p.name}`}
                          onClick={() => void forgetRemote(p.id)}
                          className="flex size-7 items-center justify-center rounded-control text-text-faint hover:bg-surface hover:text-danger"
                        >
                          <Trash2 className="size-4" aria-hidden="true" />
                        </button>
                        {p.online && p.port != null && (
                          <button
                            type="button"
                            aria-label={`Open ${p.name}`}
                            onClick={() =>
                              void browse({
                                id: p.id,
                                name: p.name,
                                host: "127.0.0.1",
                                port: p.port as number,
                              })
                            }
                            className="text-text-faint transition-colors hover:text-text"
                          >
                            <ChevronRight className="size-4" aria-hidden="true" />
                          </button>
                        )}
                      </li>
                    ))}
                  </ul>
                )}
                <div className="mt-3 border-t border-border pt-3">
                  <Button
                    variant="primary"
                    disabled={remoteBusy}
                    onClick={() => void startRemotePairing()}
                  >
                    <QrCode className="mr-1.5 size-4" aria-hidden="true" />
                    {remoteBusy ? "Preparing…" : "Pair a phone across networks"}
                  </Button>
                </div>
              </>
            )}
          </Card>

          <p className="text-xs text-text-faint">
            Music streams from your phone and plays through the desktop’s
            enhancement chain. On the same Wi‑Fi, phones appear above
            automatically; on a different network, pair once with a QR and they
            reconnect peer-to-peer over iroh.
          </p>
        </div>
  );

  return content;
}

/** PIN entry shown while pairing with a phone. */
function PinForm({
  device,
  pin,
  busy,
  onPin,
  onSubmit,
  onCancel,
}: {
  device: PhoneDevice;
  pin: string;
  busy: boolean;
  onPin: (v: string) => void;
  onSubmit: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm text-text-muted">
        Enter the 6-digit code shown on <span className="font-medium text-text">{device.name}</span>.
      </p>
      <div className="flex items-center gap-2">
        <input
          autoFocus
          value={pin}
          inputMode="numeric"
          maxLength={6}
          placeholder="000000"
          onChange={(e) => onPin(e.target.value.replace(/\D/g, "").slice(0, 6))}
          onKeyDown={(e) => {
            if (e.key === "Enter") onSubmit();
            if (e.key === "Escape") onCancel();
          }}
          className="w-36 rounded-control border border-accent/40 bg-surface px-3 py-2 text-center text-lg tracking-[0.4em] tabular-nums outline-none placeholder:text-text-faint"
        />
        <Button variant="primary" disabled={busy || pin.length < 4} onClick={onSubmit}>
          {busy ? "Pairing…" : "Pair"}
        </Button>
        <Button variant="secondary" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

/** A paired phone's library: a ranked, playable track list. */
function DeviceLibrary({
  device,
  tracks,
  loading,
  nowPlaying,
  onBack,
  onPlay,
  banner,
}: {
  device: PhoneDevice;
  tracks: PhoneTrack[];
  loading: boolean;
  nowPlaying: string | null;
  onBack: () => void;
  onPlay: (track: PhoneTrack) => void;
  banner: React.ReactNode;
}) {
  const subtitle = useMemo(
    () => `${tracks.length} track${tracks.length === 1 ? "" : "s"}`,
    [tracks.length],
  );

  return (
    <Card
      title={device.name}
      icon={Smartphone}
      actions={
        <button
          type="button"
          onClick={onBack}
          className="flex items-center gap-1 text-sm text-text-muted transition-colors hover:text-text"
        >
          <ChevronLeft className="size-4" aria-hidden="true" />
          Phones
        </button>
      }
    >
      <p className="mb-2 text-xs text-text-faint">{subtitle}</p>
      {banner && <div className="mb-3">{banner}</div>}

      {loading ? (
        <p className="text-sm text-text-muted">Loading library…</p>
      ) : tracks.length === 0 ? (
        <p className="text-sm text-text-muted">No music found on this phone.</p>
      ) : (
        <ol className="flex max-h-[60vh] flex-col overflow-y-auto">
          {tracks.map((t, i) => {
            const isPlaying = nowPlaying === t.title;
            const secs = t.durationMs != null ? Math.round(t.durationMs / 1000) : null;
            return (
              <li
                key={t.id}
                onClick={() => onPlay(t)}
                className={cn(
                  "group flex cursor-pointer items-center gap-3 rounded-control px-2 py-1.5 transition-colors hover:bg-surface-overlay",
                  isPlaying && "bg-accent-muted/40",
                )}
              >
                <span
                  className={cn(
                    "w-6 text-right text-xs tabular-nums",
                    isPlaying ? "text-accent-strong" : "text-text-faint",
                  )}
                >
                  {String(i + 1).padStart(2, "0")}
                </span>
                <div className="relative">
                  <PhoneCover deviceId={device.id} track={t} />
                  <span className="absolute inset-0 grid place-items-center rounded-md bg-black/45 opacity-0 transition-opacity group-hover:opacity-100">
                    <Play className="size-4 text-white" aria-hidden="true" />
                  </span>
                </div>
                <div className="min-w-0 flex-1">
                  <p
                    className={cn(
                      "truncate text-sm font-medium",
                      isPlaying && "text-accent-strong",
                    )}
                  >
                    {t.title}
                  </p>
                  <p className="truncate text-xs text-text-muted">
                    {t.artist ?? "—"}
                  </p>
                </div>
                <span className="w-16 shrink-0 text-right text-xs tabular-nums text-text-muted">
                  {formatTime(secs)}
                </span>
              </li>
            );
          })}
        </ol>
      )}
    </Card>
  );
}
