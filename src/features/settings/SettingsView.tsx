import { useEffect, useState } from "react";
import { AudioLines, CircleAlert, Info, KeyRound, Speaker } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { useUiStore } from "@/stores/ui";
import { useEngineStore } from "@/stores/engine";
import {
  captureVirtualAvailable,
  ipcErrorMessage,
  licenseDeactivate,
  licenseStatus,
  listOutputDevices,
  playerPlayCapture,
} from "@/lib/ipc";
import type { DeviceInfo } from "@/lib/types";

function licenseLabel(
  license: ReturnType<typeof useUiStore.getState>["license"],
): string {
  if (!license) return "—";
  if (license.kind === "licensed") return "Licensed";
  if (license.kind === "expired") return "Trial expired";
  return `Trial — ${license.daysLeft} days left`;
}

type DeviceState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; devices: DeviceInfo[] };

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between py-2 text-sm">
      <span className="text-text-muted">{label}</span>
      <span className="font-medium tabular-nums">{value}</span>
    </div>
  );
}

function OutputDevices({ state }: { state: DeviceState }) {
  if (state.status === "loading") {
    return (
      <div className="space-y-2" aria-busy="true" aria-label="Loading devices">
        {[0, 1, 2].map((i) => (
          <div
            key={i}
            className="h-9 animate-pulse rounded-control bg-surface-overlay"
          />
        ))}
      </div>
    );
  }

  if (state.status === "error") {
    return (
      <div className="flex items-center gap-2 rounded-control border border-danger/30 bg-danger/10 px-3 py-2.5 text-sm text-text">
        <CircleAlert className="size-4 shrink-0 text-danger" aria-hidden="true" />
        <span>Could not list devices: {state.message}</span>
      </div>
    );
  }

  if (state.devices.length === 0) {
    return <p className="text-sm text-text-muted">No output devices found.</p>;
  }

  return (
    <ul className="divide-y divide-border">
      {state.devices.map((device) => (
        <li
          key={device.name}
          className="flex items-center justify-between py-2.5 text-sm"
        >
          <span className="truncate">{device.name}</span>
          {device.isDefault && (
            <span className="ml-3 shrink-0 rounded-full border border-accent/40 bg-accent-muted px-2 py-0.5 text-xs text-accent-strong">
              Default
            </span>
          )}
        </li>
      ))}
    </ul>
  );
}

/**
 * Settings — the one Phase 0 view backed by live data: it shows real app
 * metadata (from `app_info`) and the system's real output devices (from
 * `audio_list_output_devices`), proving the typed IPC seam at runtime.
 */
export function SettingsView() {
  const route = routeById("settings");
  const appInfo = useUiStore((s) => s.appInfo);
  const license = useUiStore((s) => s.license);
  const setLicense = useUiStore((s) => s.setLicense);
  const stop = useEngineStore((s) => s.stop);
  const playing = useEngineStore((s) => s.playing);
  const [devices, setDevices] = useState<DeviceState>({ status: "loading" });
  const [virtualAvailable, setVirtualAvailable] = useState(false);
  const [captureError, setCaptureError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    listOutputDevices()
      .then((list) => {
        if (!cancelled) setDevices({ status: "ready", devices: list });
      })
      .catch((err: unknown) => {
        if (!cancelled)
          setDevices({ status: "error", message: ipcErrorMessage(err) });
      });
    captureVirtualAvailable()
      .then((v) => !cancelled && setVirtualAvailable(v))
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const startCapture = () => {
    setCaptureError(null);
    playerPlayCapture().catch((e) => setCaptureError(ipcErrorMessage(e)));
  };
  const deactivate = () => {
    licenseDeactivate()
      .then(() => licenseStatus())
      .then(setLicense)
      .catch(() => {});
  };

  return (
    <div className="mx-auto w-full max-w-3xl">
      <PageHeader
        icon={route.icon}
        title={route.label}
        subtitle={route.tagline}
      />
      <div className="grid gap-4">
        <Card title="About" icon={Info}>
          <div className="divide-y divide-border">
            <InfoRow label="Application" value={appInfo?.name ?? "HypeMuzik"} />
            <InfoRow label="Version" value={appInfo?.version ?? "—"} />
            <InfoRow
              label="Engine schema"
              value={appInfo ? `v${appInfo.engineSchema}` : "—"}
            />
          </div>
        </Card>

        <Card title="Output devices" icon={Speaker}>
          <OutputDevices state={devices} />
        </Card>

        <Card title="Account" icon={KeyRound}>
          <div className="divide-y divide-border">
            <InfoRow label="License (mock)" value={licenseLabel(license)} />
          </div>
          {license?.kind === "licensed" && (
            <div className="mt-3 flex justify-end">
              <Button variant="secondary" onClick={deactivate}>
                Deactivate
              </Button>
            </div>
          )}
          <p className="mt-3 text-xs text-text-faint">
            Licensing is an explicitly-marked local mock — no real DRM or
            activation server. See docs/architecture.md for the production
            contract.
          </p>
        </Card>

        <Card title="Audio capture" icon={AudioLines}>
          <div className="flex flex-col gap-3">
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0 text-sm">
                <p className="font-medium">Input capture (dev stand-in)</p>
                <p className="text-xs text-text-muted">
                  Route the default input device through the chain.
                </p>
              </div>
              <div className="flex shrink-0 gap-2">
                <Button variant="secondary" onClick={startCapture}>
                  Start
                </Button>
                {playing && (
                  <Button variant="ghost" onClick={() => void stop()}>
                    Stop
                  </Button>
                )}
              </div>
            </div>
            {captureError && (
              <p className="text-sm text-danger">{captureError}</p>
            )}
            <div className="flex items-start gap-2 rounded-control border border-border bg-surface px-3 py-2 text-xs text-text-muted">
              <CircleAlert
                className="mt-0.5 size-3.5 shrink-0 text-text-faint"
                aria-hidden="true"
              />
              <span>
                System-wide capture (intercepting other apps):{" "}
                <span className="text-text">
                  {virtualAvailable ? "available" : "not installed"}
                </span>
                . It requires a signed virtual audio driver — see
                docs/audio-driver.md.
              </span>
            </div>
          </div>
        </Card>
      </div>
    </div>
  );
}
