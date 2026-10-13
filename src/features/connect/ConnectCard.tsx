import { useCallback, useEffect, useRef, useState } from "react";
import {
  Cast,
  Globe,
  Laptop,
  LoaderCircle,
  QrCode,
  RefreshCw,
  Smartphone,
  Trash2,
} from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { Switch } from "@/components/Switch";
import {
  connectDevices,
  connectLinkApprove,
  connectLinkCancel,
  connectLinkDeny,
  connectLinkPoll,
  connectLinkPreview,
  connectLinkStart,
  connectRevoke,
  connectSetEnabled,
  ipcErrorMessage,
} from "@/lib/ipc";
import type { ConnectLinkPreview, ConnectLinkTicket } from "@/lib/ipc";
import { cn } from "@/lib/cn";
import { useAccountStore } from "@/stores/account";
import { toast } from "@/stores/toast";
import {
  describeConnection,
  deviceSubtitle,
  formatCode,
  platformLabel,
  secondsUntil,
  type Tone,
} from "./describe";
import type { DeviceInfo, DeviceKind } from "./protocol";
import { useConnectStore } from "./store";

const POLL_MS = 2_000;

const DOT: Record<Tone, string> = {
  ok: "bg-success",
  busy: "bg-accent animate-pulse",
  warn: "bg-warning",
  off: "bg-border-strong",
};

const KIND_ICON: Record<DeviceKind, typeof Smartphone> = {
  mobile: Smartphone,
  desktop: Laptop,
  web: Globe,
};

type Flow = "none" | "phone" | "browser";

/** The toast for an approved phone link: names the phone when the device list
 *  knows it (the most recently seen mobile device), else stays generic. */
export function linkedMessage(devices: DeviceInfo[]): string {
  const phone = devices
    .filter((d) => d.kind === "mobile")
    .sort((a, b) => b.lastSeenAt.localeCompare(a.lastSeenAt))[0];
  return phone
    ? `Linked ${phone.name} — you can now play on it from here, and it can play on this computer.`
    : "Linked — this desktop now shares your phone's account.";
}

/** Re-read the device list into the store (the socket keeps it live after). */
async function refreshDevices(): Promise<void> {
  const { devices, activeDeviceId } = await connectDevices();
  useConnectStore.getState().setDevices(devices, activeDeviceId);
}

/**
 * Hype Connect in Settings → Devices: the hub connection, linking (this
 * desktop to a phone, or a browser to this desktop), and the account's
 * linked devices.
 */
export function ConnectCard() {
  const status = useConnectStore((s) => s.status);
  const [flow, setFlow] = useState<Flow>("none");
  const [toggling, setToggling] = useState(false);
  const view = describeConnection(status);
  const enabled = status?.enabled ?? false;

  const setEnabled = async (on: boolean) => {
    setToggling(true);
    try {
      useConnectStore.getState().setStatus(await connectSetEnabled(on));
    } catch (e) {
      toast.error(`Hype Connect: ${ipcErrorMessage(e)}`);
    } finally {
      setToggling(false);
    }
  };

  return (
    <Card
      title="Hype Connect"
      icon={Cast}
      actions={
        <Switch
          checked={enabled}
          onChange={(on) => !toggling && void setEnabled(on)}
          label="Hype Connect"
        />
      }
    >
      <div className="flex items-start gap-3">
        <span
          className={cn("mt-1.5 size-2.5 shrink-0 rounded-full", DOT[view.tone])}
          aria-hidden="true"
        />
        <div className="min-w-0 flex-1" role="status" aria-live="polite">
          <p className="text-sm font-medium">{view.title}</p>
          <p className="mt-0.5 text-xs text-text-muted">{view.detail}</p>
        </div>
        {view.canReconnect && (
          <Button variant="secondary" disabled={toggling} onClick={() => void setEnabled(true)}>
            <RefreshCw className="size-4" aria-hidden="true" />
            Reconnect
          </Button>
        )}
      </div>

      {enabled && (
        <>
          {flow === "phone" && <PhoneLink onClose={() => setFlow("none")} />}
          {flow === "browser" && <BrowserLink onClose={() => setFlow("none")} />}
          {flow === "none" && (
            <div className="mt-4 flex flex-wrap gap-2">
              <Button variant="primary" onClick={() => setFlow("phone")}>
                <QrCode className="size-4" aria-hidden="true" />
                Link with your phone
              </Button>
              <Button variant="secondary" onClick={() => setFlow("browser")}>
                <Globe className="size-4" aria-hidden="true" />
                Link a browser
              </Button>
            </div>
          )}
          <LinkedDevices selfId={status?.deviceId ?? ""} />
        </>
      )}
    </Card>
  );
}

/** "Link with your phone": show a QR + code, poll until the phone answers. */
function PhoneLink({ onClose }: { onClose: () => void }) {
  const [ticket, setTicket] = useState<ConnectLinkTicket | null>(null);
  const [outcome, setOutcome] = useState<"denied" | "expired" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let active = true;
    setTicket(null);
    setOutcome(null);
    setError(null);
    connectLinkStart()
      .then((t) => active && setTicket(t))
      .catch((e) => active && setError(ipcErrorMessage(e)));
    return () => {
      active = false;
    };
  }, [attempt]);

  // Poll the claim while the code is live; tick the countdown alongside.
  useEffect(() => {
    if (!ticket || outcome) return;
    let active = true;
    let inFlight = false;
    const timer = setInterval(() => {
      setNow(Date.now());
      if (inFlight) return;
      inFlight = true;
      connectLinkPoll()
        .then(({ status }) => {
          if (!active || status === "pending") return;
          if (status === "approved") {
            void useAccountStore.getState().refresh();
            onClose();
            // Name the phone once the device list shows it. The phone — not
            // this desktop — asks where music should play; remote mode flips
            // on by itself when that choice lands.
            void refreshDevices()
              .catch(() => {})
              .then(() => toast.success(linkedMessage(useConnectStore.getState().devices)));
          } else {
            setOutcome(status);
          }
        })
        .catch((e) => active && setError(ipcErrorMessage(e)))
        .finally(() => {
          inFlight = false;
        });
    }, POLL_MS);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [ticket, outcome, onClose]);

  const close = () => {
    void connectLinkCancel().catch(() => {});
    onClose();
  };

  const left = ticket ? secondsUntil(ticket.expiresAt, now) : 0;

  return (
    <div className="mt-4 rounded-card border border-border bg-surface p-4">
      {error ? (
        <p className="text-sm text-danger">{error}</p>
      ) : outcome ? (
        <p className="text-sm text-text-muted">
          {outcome === "denied"
            ? "The phone declined the link."
            : "That code expired before it was used."}
        </p>
      ) : !ticket ? (
        <p className="flex items-center gap-2 text-sm text-text-muted">
          <LoaderCircle className="size-4 animate-spin" aria-hidden="true" />
          Getting a code…
        </p>
      ) : (
        <div className="flex flex-col items-center gap-3 text-center">
          <p className="text-sm text-text-muted">
            In Hype Muzik on your phone, scan this code — or type the numbers.
          </p>
          <div className="rounded-xl bg-white p-3">
            <QRCodeSVG value={ticket.qr} size={176} marginSize={1} />
          </div>
          <p
            className="font-mono text-2xl font-semibold tracking-[0.2em] tabular-nums"
            aria-label={`Code ${ticket.code.split("").join(" ")}`}
          >
            {formatCode(ticket.code)}
          </p>
          <p className="text-xs text-text-faint">
            {left > 0
              ? `Expires in ${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`
              : "Expired"}
            {" · "}Linking merges this desktop into your phone's account.
          </p>
        </div>
      )}
      <div className="mt-4 flex justify-center gap-2">
        {(outcome || error) && (
          <Button variant="primary" onClick={() => setAttempt((n) => n + 1)}>
            New code
          </Button>
        )}
        <Button variant="secondary" onClick={close}>
          {outcome || error ? "Close" : "Cancel"}
        </Button>
      </div>
    </div>
  );
}

/** "Link a browser": the code the web shows → who's asking → approve/deny. */
function BrowserLink({ onClose }: { onClose: () => void }) {
  const [code, setCode] = useState("");
  const [preview, setPreview] = useState<ConnectLinkPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => inputRef.current?.focus(), []);

  const run = async (task: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await task();
    } catch (e) {
      setError(ipcErrorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const lookUp = () => run(async () => setPreview(await connectLinkPreview(code)));

  const approve = () =>
    run(async () => {
      await connectLinkApprove(code);
      toast.success(`Linked ${preview?.name ?? "the browser"}.`);
      void refreshDevices().catch(() => {});
      onClose();
    });

  const deny = () =>
    run(async () => {
      await connectLinkDeny(code);
      onClose();
    });

  const Icon = preview ? KIND_ICON[preview.kind] : Globe;
  const platform = preview ? platformLabel(preview.platform) : null;

  return (
    <div className="mt-4 rounded-card border border-border bg-surface p-4">
      {preview ? (
        <div className="flex flex-col items-center gap-3 text-center">
          <span className="grid size-12 place-items-center rounded-full bg-accent/15 text-accent">
            <Icon className="size-6" aria-hidden="true" />
          </span>
          <p className="text-base font-medium">Link {preview.name}?</p>
          <p className="text-xs text-text-muted">
            {platform && !preview.name.includes(platform) ? `${platform} · ` : ""}
            It will be able to see and play music on your devices.
          </p>
          <div className="flex gap-2">
            <Button variant="primary" disabled={busy} onClick={() => void approve()}>
              Approve
            </Button>
            <Button variant="secondary" disabled={busy} onClick={() => void deny()}>
              Deny
            </Button>
          </div>
        </div>
      ) : (
        <form
          className="flex flex-col gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (code.length === 6) void lookUp();
          }}
        >
          <label htmlFor="connect-code" className="text-sm text-text-muted">
            Enter the 6-digit code shown in your browser.
          </label>
          <div className="flex gap-2">
            <input
              id="connect-code"
              ref={inputRef}
              value={code}
              onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
              inputMode="numeric"
              autoComplete="one-time-code"
              placeholder="000000"
              className="w-36 rounded-control border border-border bg-surface-raised px-3 py-2 text-center font-mono text-lg tracking-[0.3em] tabular-nums outline-none placeholder:text-text-faint focus:border-accent/50"
            />
            <Button type="submit" variant="primary" disabled={busy || code.length !== 6}>
              {busy ? "Checking…" : "Continue"}
            </Button>
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
          </div>
        </form>
      )}
      {error && <p className="mt-3 text-sm text-danger">{error}</p>}
    </div>
  );
}

/** The account's linked devices, with Revoke for every one but this desktop. */
function LinkedDevices({ selfId }: { selfId: string }) {
  const devices = useConnectStore((s) => s.devices);
  const activeId = useConnectStore((s) => s.activeDeviceId);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const now = Date.now();

  const load = useCallback(() => {
    refreshDevices()
      .then(() => setError(null))
      .catch((e) => setError(ipcErrorMessage(e)));
  }, []);

  useEffect(load, [load]);

  // A second click within a few seconds confirms; otherwise it disarms.
  useEffect(() => {
    if (!confirming) return;
    const t = setTimeout(() => setConfirming(null), 4_000);
    return () => clearTimeout(t);
  }, [confirming]);

  const revoke = async (d: DeviceInfo) => {
    if (confirming !== d.id) {
      setConfirming(d.id);
      return;
    }
    setConfirming(null);
    try {
      await connectRevoke(d.id);
      toast.info(`Unlinked ${d.name}.`);
      load();
    } catch (e) {
      toast.error(ipcErrorMessage(e));
    }
  };

  // This desktop first, then online before offline.
  const sorted = [...devices].sort(
    (a, b) =>
      Number(b.id === selfId) - Number(a.id === selfId) || Number(b.online) - Number(a.online),
  );

  return (
    <div className="mt-5 border-t border-border pt-3">
      <div className="flex items-center justify-between">
        <p className="text-xs font-medium uppercase tracking-wide text-text-faint">
          Linked devices
        </p>
        <button
          type="button"
          aria-label="Refresh linked devices"
          onClick={load}
          className="text-text-muted transition-colors hover:text-text"
        >
          <RefreshCw className="size-3.5" aria-hidden="true" />
        </button>
      </div>
      {error ? (
        <p className="py-2 text-sm text-text-muted">{error}</p>
      ) : sorted.length === 0 ? (
        <p className="py-2 text-sm text-text-muted">
          Nothing linked yet. Link a browser to play music here from anywhere.
        </p>
      ) : (
        <ul className="divide-y divide-border">
          {sorted.map((d) => {
            const Icon = KIND_ICON[d.kind];
            const self = d.id === selfId;
            return (
              <li key={d.id} className="flex items-center gap-3 py-2.5">
                <Icon
                  className={cn("size-5 shrink-0", d.online ? "text-text-muted" : "text-text-faint")}
                  aria-hidden="true"
                />
                <span className="min-w-0 flex-1">
                  <span
                    className={cn(
                      "block truncate text-sm font-medium",
                      !d.online && "text-text-muted",
                    )}
                  >
                    {d.name}
                  </span>
                  <span className="block truncate text-xs text-text-faint">
                    {deviceSubtitle(d, now)}
                  </span>
                </span>
                {d.id === activeId && (
                  <span className="rounded-control bg-success/15 px-2 py-0.5 text-xs text-success">
                    {self ? "Playing here" : "Playing now"}
                  </span>
                )}
                {self ? (
                  <span className="rounded-control bg-surface px-2 py-0.5 text-xs text-text-faint">
                    This desktop
                  </span>
                ) : (
                  <button
                    type="button"
                    aria-label={confirming === d.id ? `Confirm unlinking ${d.name}` : `Unlink ${d.name}`}
                    onClick={() => void revoke(d)}
                    className={cn(
                      "flex h-7 items-center justify-center gap-1 rounded-control px-1.5 text-xs transition-colors",
                      confirming === d.id
                        ? "bg-danger/15 text-danger"
                        : "text-text-faint hover:bg-surface hover:text-danger",
                    )}
                  >
                    <Trash2 className="size-4" aria-hidden="true" />
                    {confirming === d.id && "Unlink?"}
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
