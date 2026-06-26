import { useCallback, useEffect, useState } from "react";
import {
  ChevronRight,
  CircleAlert,
  Cloud,
  Folder,
  HardDrive,
  Music,
  Plus,
  RefreshCw,
  UserRound,
} from "lucide-react";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { useEngineStore } from "@/stores/engine";
import { useMusicLibraryStore } from "@/stores/musicLibrary";
import {
  cloudConnect,
  cloudDisconnect,
  cloudList,
  cloudStatus,
  ipcErrorMessage,
} from "@/lib/ipc";
import type {
  CloudAccount,
  CloudEntry,
  CloudProvider,
  CloudStatus,
} from "@/lib/types";
import { formatBytes } from "@/lib/format";
import { cn } from "@/lib/cn";

interface ProviderMeta {
  id: CloudProvider;
  name: string;
  configured: (s: CloudStatus) => boolean;
}

const PROVIDERS: readonly ProviderMeta[] = [
  {
    id: "googleDrive",
    name: "Google Drive",
    configured: (s) => s.googleConfigured,
  },
  {
    id: "dropbox",
    name: "Dropbox",
    configured: (s) => s.dropboxConfigured,
  },
];

/** A breadcrumb level: which account folder we're inside. */
interface Crumb {
  accountId: string;
  id: string; // "" = account root
  name: string;
}

/** Cloud accounts (Google Drive / Dropbox) — connect/disconnect and browse +
 *  play folders. Multiple accounts per provider are supported (e.g. several
 *  Google accounts); the Player merges every connected account's songs into its
 *  one unified "Cloud" source. */
export function CloudView() {
  const nowPlaying = useEngineStore((s) => s.nowPlaying);
  const playCloudList = useEngineStore((s) => s.playCloudList);

  const [status, setStatus] = useState<CloudStatus | null>(null);
  const [stack, setStack] = useState<Crumb[]>([]);
  const [entries, setEntries] = useState<CloudEntry[]>([]);
  // Which provider is mid-connect (for its button), and which account is being
  // removed (so its Disconnect shows progress).
  const [connecting, setConnecting] = useState<CloudProvider | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  /** Play `e` within the current folder so next/prev walk its audio files. */
  const playEntry = (e: CloudEntry) =>
    playCloudList(
      entries,
      entries.findIndex((x) => x.id === e.id),
    );

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await cloudStatus());
    } catch (e) {
      setError(ipcErrorMessage(e));
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  const here = stack[stack.length - 1];

  // Load the current folder's contents whenever we navigate.
  const loadFolder = useCallback(async (crumb: Crumb | undefined) => {
    if (!crumb) {
      setEntries([]);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      setEntries(await cloudList(crumb.accountId, crumb.id));
    } catch (e) {
      setError(ipcErrorMessage(e));
      setEntries([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadFolder(here);
  }, [here, loadFolder]);

  const connect = async (provider: CloudProvider) => {
    setError(null);
    setConnecting(provider);
    try {
      await cloudConnect(provider);
      await refreshStatus();
      // The unified library caches cloud tracks; tell it to reload now that the
      // set of connected accounts changed (otherwise it'd keep the old list).
      useMusicLibraryStore.getState().invalidateCloud();
    } catch (e) {
      setError(ipcErrorMessage(e));
    } finally {
      setConnecting(null);
    }
  };

  const disconnect = async (accountId: string) => {
    setRemoving(accountId);
    await cloudDisconnect(accountId).catch(() => {});
    // Leave any folder we were browsing in that account.
    setStack((s) => (s[0]?.accountId === accountId ? [] : s));
    await refreshStatus();
    useMusicLibraryStore.getState().invalidateCloud();
    setRemoving(null);
  };

  const accounts = status?.accounts ?? [];
  const accountsFor = (provider: CloudProvider): CloudAccount[] =>
    accounts.filter((a) => a.provider === provider);

  return (
    <div className="flex flex-col gap-4">
      {/* Accounts */}
      <Card title="Accounts" icon={Cloud}>
        <div className="flex flex-col gap-5">
          {PROVIDERS.map((p) => {
            const isConfigured = status ? p.configured(status) : true;
            const mine = accountsFor(p.id);
            return (
              <div key={p.id} className="flex flex-col gap-2.5">
                <div className="flex items-center justify-between gap-3">
                  <div className="flex items-center gap-2 text-sm">
                    <HardDrive className="size-4 text-text-muted" aria-hidden="true" />
                    <span className="font-medium">{p.name}</span>
                    {mine.length > 0 && (
                      <span className="rounded-control bg-success/15 px-2 py-0.5 text-xs text-success">
                        {mine.length} connected
                      </span>
                    )}
                    {!isConfigured && (
                      <span className="text-xs text-text-faint">not configured</span>
                    )}
                  </div>
                  <Button
                    variant={mine.length > 0 ? "secondary" : "primary"}
                    disabled={!isConfigured || connecting !== null}
                    onClick={() => void connect(p.id)}
                  >
                    {connecting === p.id ? (
                      "Connecting…"
                    ) : (
                      <>
                        <Plus className="size-4" aria-hidden="true" />
                        {mine.length > 0 ? "Add account" : "Connect"}
                      </>
                    )}
                  </Button>
                </div>

                {/* Connected accounts for this provider, each removable. */}
                {mine.length > 0 && (
                  <ul className="flex flex-col gap-1.5 border-l border-border pl-3">
                    {mine.map((a) => (
                      <li
                        key={a.id}
                        className="flex items-center justify-between gap-3"
                      >
                        <span className="flex min-w-0 items-center gap-2 text-sm">
                          <UserRound
                            className="size-3.5 shrink-0 text-text-faint"
                            aria-hidden="true"
                          />
                          <span className="truncate">{a.label}</span>
                        </span>
                        <button
                          type="button"
                          onClick={() => void disconnect(a.id)}
                          disabled={removing === a.id}
                          className="shrink-0 text-xs text-text-muted transition-colors hover:text-danger disabled:opacity-50"
                        >
                          {removing === a.id ? "Removing…" : "Disconnect"}
                        </button>
                      </li>
                    ))}
                  </ul>
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

      {/* Browser */}
      {accounts.length > 0 && (
        <Card
          title="Browse"
          icon={Folder}
          actions={
            here && (
              <button
                type="button"
                aria-label="Refresh"
                onClick={() => void loadFolder(here)}
                className="text-text-muted transition-colors hover:text-text"
              >
                <RefreshCw
                  className={cn("size-4", loading && "animate-spin")}
                  aria-hidden="true"
                />
              </button>
            )
          }
        >
          {/* Breadcrumb */}
          <div className="mb-3 flex flex-wrap items-center gap-1 text-sm">
            <button
              type="button"
              onClick={() => setStack([])}
              className={cn(
                "rounded-control px-1.5 py-0.5 transition-colors hover:text-text",
                stack.length === 0 ? "text-text" : "text-text-muted",
              )}
            >
              Cloud
            </button>
            {stack.map((c, i) => (
              <span key={`${c.accountId}:${c.id}`} className="flex items-center gap-1">
                <ChevronRight className="size-3.5 text-text-faint" aria-hidden="true" />
                <button
                  type="button"
                  onClick={() => setStack((s) => s.slice(0, i + 1))}
                  className={cn(
                    "max-w-[12rem] truncate rounded-control px-1.5 py-0.5 transition-colors hover:text-text",
                    i === stack.length - 1 ? "text-text" : "text-text-muted",
                  )}
                >
                  {c.name}
                </button>
              </span>
            ))}
          </div>

          {/* At the root: list connected accounts as folders. */}
          {!here ? (
            <ul className="divide-y divide-border">
              {accounts.map((a) => (
                <li key={a.id}>
                  <button
                    type="button"
                    onClick={() =>
                      setStack([{ accountId: a.id, id: "", name: a.label }])
                    }
                    className="flex w-full items-center gap-3 py-2.5 text-left transition-colors hover:text-accent-strong"
                  >
                    <HardDrive className="size-4 shrink-0 text-text-muted" aria-hidden="true" />
                    <span className="flex-1 truncate text-sm font-medium">{a.label}</span>
                    <ChevronRight className="size-4 text-text-faint" aria-hidden="true" />
                  </button>
                </li>
              ))}
            </ul>
          ) : loading && entries.length === 0 ? (
            <p className="text-sm text-text-muted">Loading…</p>
          ) : entries.length === 0 ? (
            <p className="text-sm text-text-muted">This folder has no music.</p>
          ) : (
            <ul className="divide-y divide-border">
              {entries.map((e) =>
                e.isFolder ? (
                  <li key={`${e.accountId}:${e.id}`}>
                    <button
                      type="button"
                      onClick={() =>
                        setStack((s) => [
                          ...s,
                          { accountId: e.accountId, id: e.id, name: e.name },
                        ])
                      }
                      className="flex w-full items-center gap-3 py-2.5 text-left transition-colors hover:text-accent-strong"
                    >
                      <Folder className="size-4 shrink-0 text-text-muted" aria-hidden="true" />
                      <span className="flex-1 truncate text-sm">{e.name}</span>
                      <ChevronRight className="size-4 text-text-faint" aria-hidden="true" />
                    </button>
                  </li>
                ) : (
                  <li
                    key={`${e.accountId}:${e.id}`}
                    className="flex items-center gap-3 py-2"
                  >
                    <button
                      type="button"
                      aria-label={`Play ${e.name}`}
                      onClick={() => playEntry(e)}
                      className={cn(
                        "transition-colors hover:text-accent-strong",
                        nowPlaying === e.name ? "text-accent-strong" : "text-text-muted",
                      )}
                    >
                      <Music className="size-4" aria-hidden="true" />
                    </button>
                    <button
                      type="button"
                      onClick={() => playEntry(e)}
                      className={cn(
                        "min-w-0 flex-1 truncate text-left text-sm transition-colors hover:text-text",
                        nowPlaying === e.name && "text-accent-strong",
                      )}
                    >
                      {e.name}
                    </button>
                    <span className="shrink-0 text-xs tabular-nums text-text-faint">
                      {formatBytes(e.size)}
                    </span>
                  </li>
                ),
              )}
            </ul>
          )}
        </Card>
      )}
    </div>
  );
}
