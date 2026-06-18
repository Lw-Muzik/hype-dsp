import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  CircleAlert,
  Play,
  RefreshCw,
  Smartphone,
  Trash2,
  Wifi,
} from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import {
  ipcErrorMessage,
  linkDiscover,
  linkLibrary,
  linkPair,
  linkPaired,
  linkUnpair,
} from "@/lib/ipc";
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

export function DevicesView() {
  const route = routeById("phone");
  const nowPlaying = useEngineStore((s) => s.nowPlaying);
  const playPhone = useEngineStore((s) => s.playPhone);

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

  /** Load paired phones + browse the LAN, merged (paired first). */
  const scan = useCallback(async () => {
    setScanning(true);
    setError(null);
    try {
      const paired = await linkPaired();
      const pairedSet = new Set(paired.map((d) => d.id));
      setPairedIds(pairedSet);
      let merged = paired;
      try {
        const found = await linkDiscover();
        const extra = found.filter((d) => !pairedSet.has(d.id));
        merged = [...paired, ...extra];
      } catch (e) {
        // Discovery can fail (no network); still show paired devices.
        setError(ipcErrorMessage(e));
      }
      setDevices(merged);
    } catch (e) {
      setError(ipcErrorMessage(e));
    } finally {
      setScanning(false);
    }
  }, []);

  useEffect(() => {
    void scan();
  }, [scan]);

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

  const unpair = async (device: PhoneDevice) => {
    await linkUnpair(device.id).catch(() => {});
    if (open?.id === device.id) {
      setOpen(null);
      setTracks([]);
    }
    await scan();
  };

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

  return (
    <div className="mx-auto w-full max-w-3xl">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      {open ? (
        <DeviceLibrary
          device={open}
          tracks={tracks}
          loading={loading}
          nowPlaying={nowPlaying}
          onBack={back}
          onPlay={(t) => playPhone(open, t)}
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
            ) : devices.length === 0 ? (
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
                {devices.map((d) => {
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
            {!pairing && errorBanner && <div className="mt-3">{errorBanner}</div>}
          </Card>

          <p className="text-xs text-text-faint">
            Music streams from your phone over the local network and plays through
            the desktop’s enhancement chain. Same Wi‑Fi only.
          </p>
        </div>
      )}
    </div>
  );
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
                  <Thumb seed={t.album?.trim() || t.title} label={t.title} />
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
