import { useCallback, useEffect, useState } from "react";
import { CircleAlert, Cloud, HardDrive, Play, RefreshCw } from "lucide-react";
import { routeById } from "@/app/routes";
import { PageHeader } from "@/components/PageHeader";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import {
  cloudConnect,
  cloudDisconnect,
  cloudList,
  cloudStatus,
  ipcErrorMessage,
} from "@/lib/ipc";
import type { CloudFile, CloudProvider, CloudStatus } from "@/lib/types";
import { formatBytes } from "@/lib/format";

interface ProviderMeta {
  id: CloudProvider;
  name: string;
  connected: (s: CloudStatus) => boolean;
  configured: (s: CloudStatus) => boolean;
}

const PROVIDERS: readonly ProviderMeta[] = [
  {
    id: "googleDrive",
    name: "Google Drive",
    connected: (s) => s.googleConnected,
    configured: (s) => s.googleConfigured,
  },
  {
    id: "dropbox",
    name: "Dropbox",
    connected: (s) => s.dropboxConnected,
    configured: (s) => s.dropboxConfigured,
  },
];

export function CloudView() {
  const route = routeById("cloud");
  const nowPlaying = useEngineStore((s) => s.nowPlaying);
  const playCloud = useEngineStore((s) => s.playCloud);

  const [status, setStatus] = useState<CloudStatus | null>(null);
  const [files, setFiles] = useState<CloudFile[]>([]);
  const [busy, setBusy] = useState<CloudProvider | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await cloudStatus());
    } catch (e) {
      setError(ipcErrorMessage(e));
    }
  }, []);

  const loadFiles = useCallback(async (s: CloudStatus) => {
    const active = PROVIDERS.filter((p) => p.connected(s));
    if (active.length === 0) {
      setFiles([]);
      return;
    }
    setLoading(true);
    try {
      const lists = await Promise.all(
        active.map((p) => cloudList(p.id).catch(() => [] as CloudFile[])),
      );
      setFiles(lists.flat());
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  useEffect(() => {
    if (status) void loadFiles(status);
  }, [status, loadFiles]);

  const connect = async (provider: CloudProvider) => {
    setError(null);
    setBusy(provider);
    try {
      await cloudConnect(provider);
      await refreshStatus();
    } catch (e) {
      setError(ipcErrorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const disconnect = async (provider: CloudProvider) => {
    await cloudDisconnect(provider).catch(() => {});
    await refreshStatus();
  };

  // Group files by their folder for display.
  const byFolder = new Map<string, CloudFile[]>();
  for (const f of files) {
    const k = `${f.provider}:${f.folder}`;
    (byFolder.get(k) ?? byFolder.set(k, []).get(k)!).push(f);
  }
  const folders = [...byFolder.entries()].sort(([a], [b]) => a.localeCompare(b));
  const anyConnected = status
    ? PROVIDERS.some((p) => p.connected(status))
    : false;

  return (
    <div className="mx-auto w-full max-w-3xl">
      <PageHeader icon={route.icon} title={route.label} subtitle={route.tagline} />

      <div className="flex flex-col gap-4">
        {/* Accounts */}
        <Card title="Accounts" icon={Cloud}>
          <div className="flex flex-col gap-3">
            {PROVIDERS.map((p) => {
              const connected = status ? p.connected(status) : false;
              const configured = status ? p.configured(status) : true;
              return (
                <div key={p.id} className="flex items-center justify-between gap-3">
                  <div className="flex items-center gap-2 text-sm">
                    <HardDrive className="size-4 text-text-muted" aria-hidden="true" />
                    <span className="font-medium">{p.name}</span>
                    {connected && (
                      <span className="rounded-control bg-success/15 px-2 py-0.5 text-xs text-success">
                        Connected
                      </span>
                    )}
                    {!configured && (
                      <span className="text-xs text-text-faint">not configured</span>
                    )}
                  </div>
                  {connected ? (
                    <Button variant="secondary" onClick={() => void disconnect(p.id)}>
                      Disconnect
                    </Button>
                  ) : (
                    <Button
                      variant="primary"
                      disabled={!configured || busy !== null}
                      onClick={() => void connect(p.id)}
                    >
                      {busy === p.id ? "Connecting…" : "Connect"}
                    </Button>
                  )}
                </div>
              );
            })}
          </div>
          {error && (
            <div className="mt-3 flex items-start gap-2 rounded-control border border-danger/30 bg-danger/10 px-3 py-2 text-sm">
              <CircleAlert className="mt-0.5 size-4 shrink-0 text-danger" aria-hidden="true" />
              <span>{error}</span>
            </div>
          )}
          {status && (!status.googleConfigured || !status.dropboxConfigured) && (
            <p className="mt-3 text-xs text-text-faint">
              Set up desktop OAuth credentials to enable connecting — see
              docs/cloud-setup.md.
            </p>
          )}
        </Card>

        {/* Library */}
        {anyConnected && (
          <Card
            title="Music"
            icon={Cloud}
            actions={
              <button
                type="button"
                aria-label="Refresh"
                onClick={() => status && void loadFiles(status)}
                className="text-text-muted transition-colors hover:text-text"
              >
                <RefreshCw
                  className={`size-4 ${loading ? "animate-spin" : ""}`}
                  aria-hidden="true"
                />
              </button>
            }
          >
            {loading && files.length === 0 ? (
              <p className="text-sm text-text-muted">Loading your cloud music…</p>
            ) : files.length === 0 ? (
              <p className="text-sm text-text-muted">No audio files found.</p>
            ) : (
              <div className="flex flex-col gap-4">
                {folders.map(([key, items]) => (
                  <div key={key}>
                    <p className="mb-1 text-xs font-medium uppercase tracking-wider text-text-faint">
                      {items[0]?.folder ?? ""}
                    </p>
                    <ul className="divide-y divide-border">
                      {items.map((f) => {
                        const active = nowPlaying === f.name;
                        return (
                          <li
                            key={`${f.provider}:${f.id}`}
                            className="flex items-center gap-3 py-2"
                          >
                            <button
                              type="button"
                              aria-label={`Play ${f.name}`}
                              onClick={() => playCloud(f)}
                              className="text-text-muted transition-colors hover:text-accent-strong"
                            >
                              <Play className="size-4" aria-hidden="true" />
                            </button>
                            <span
                              className={`min-w-0 flex-1 truncate text-sm ${active ? "text-accent-strong" : ""}`}
                            >
                              {f.name}
                            </span>
                            <span className="shrink-0 text-xs tabular-nums text-text-faint">
                              {formatBytes(f.size)}
                            </span>
                          </li>
                        );
                      })}
                    </ul>
                  </div>
                ))}
              </div>
            )}
          </Card>
        )}
      </div>
    </div>
  );
}
