import { useCallback, useEffect, useState } from "react";
import {
  CircleAlert,
  CircleCheck,
  FolderOpen,
  Info,
  ListMusic,
  LogOut,
  SquarePlay,
  Terminal,
} from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { Card } from "@/components/Card";
import { Button } from "@/components/Button";
import { useMusicLibraryStore } from "@/stores/musicLibrary";
import { toast } from "@/stores/toast";
import {
  ipcErrorMessage,
  isIpcError,
  ytmusicDownloadDir,
  ytmusicSetDownloadDir,
  ytmusicSignIn,
  ytmusicSignOut,
  ytmusicStatus,
} from "@/lib/ipc";
import type { YtMusicStatus } from "@/lib/types";
import { HOST_OS } from "@/lib/platform";

/** How to install yt-dlp, per OS. It isn't bundled: it needs updating far more
 *  often than this app ships, so it's the user's package manager that keeps it
 *  working against YouTube's changes. */
const YTDLP_INSTALL: Record<typeof HOST_OS, { command: string; note: string }> = {
  mac: { command: "brew install yt-dlp", note: "with Homebrew" },
  windows: { command: "winget install yt-dlp", note: "with winget" },
  linux: {
    command: "pipx install yt-dlp",
    note: "with pipx, or your distro's package manager",
  },
};

/**
 * YouTube Music — sign in, check the tooling, and choose where downloads land.
 * The Player merges the signed-in account's playlists into its unified library
 * (grouped under Folders), so this panel is only the connect flow, mirroring
 * how `CloudView` and `DevicesView` sit in Settings.
 */
export function YtMusicView() {
  const invalidateYtMusic = useMusicLibraryStore((s) => s.invalidateYtMusic);

  const [status, setStatus] = useState<YtMusicStatus | null>(null);
  const [dir, setDir] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await ytmusicStatus());
    } catch (e) {
      setError(ipcErrorMessage(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    ytmusicDownloadDir()
      .then(setDir)
      .catch(() => {});
  }, [refresh]);

  const signIn = async () => {
    setError(null);
    setBusy(true);
    try {
      setStatus(await ytmusicSignIn());
      // The library caches YT Music tracks; tell it to reload now that there's
      // an account (otherwise it'd keep showing the signed-out empty set).
      invalidateYtMusic();
      toast.success("Signed in to YouTube Music.");
    } catch (e) {
      // Closing the window is a choice, not a failure — don't shout about it.
      if (isIpcError(e) && e.code === "cancelled") return;
      setError(ipcErrorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const signOut = async () => {
    setBusy(true);
    await ytmusicSignOut().catch((e) => setError(ipcErrorMessage(e)));
    await refresh();
    invalidateYtMusic();
    setBusy(false);
  };

  const pickDir = async () => {
    let picked: string | string[] | null;
    try {
      picked = await open({ directory: true, multiple: false });
    } catch {
      return; // dialog cancelled
    }
    if (typeof picked !== "string") return;
    try {
      setDir(await ytmusicSetDownloadDir(picked));
    } catch (e) {
      toast.error(`Couldn't use that folder: ${ipcErrorMessage(e)}`);
    }
  };

  const resetDir = async () => {
    try {
      setDir(await ytmusicSetDownloadDir(null));
    } catch (e) {
      toast.error(`Couldn't reset the folder: ${ipcErrorMessage(e)}`);
    }
  };

  const signedIn = status?.signedIn === true;
  const ytdlp = status?.ytdlp;
  const install = YTDLP_INSTALL[HOST_OS];

  return (
    <div className="flex flex-col gap-4">
      <Card title="YouTube Music" icon={SquarePlay}>
        <div className="flex flex-col gap-4">
          <div className="flex items-center justify-between gap-3">
            <div className="flex min-w-0 items-center gap-2 text-sm">
              <ListMusic className="size-4 shrink-0 text-text-muted" aria-hidden="true" />
              <span className="font-medium">Account</span>
              {signedIn && (
                <span className="rounded-control bg-success/15 px-2 py-0.5 text-xs text-success">
                  Signed in
                </span>
              )}
            </div>
            {signedIn ? (
              <Button variant="ghost" disabled={busy} onClick={() => void signOut()}>
                <LogOut className="size-4" aria-hidden="true" />
                Sign out
              </Button>
            ) : (
              <Button variant="primary" disabled={busy} onClick={() => void signIn()}>
                {busy ? "Waiting for sign-in…" : "Sign in"}
              </Button>
            )}
          </div>

          <p className="text-sm text-text-muted">
            {signedIn
              ? "Your playlists are in the Library under the YouTube source — each one groups under Folders. Tracks stream through the enhancement chain like everything else."
              : "Sign in to browse your playlists and play them through the enhancement chain. A window opens on Google's sign-in; nothing is stored beyond the session cookies."}
          </p>

          {error && (
            <div className="flex items-start gap-2 rounded-control border border-danger/30 bg-danger/10 px-3 py-2 text-sm">
              <CircleAlert className="mt-0.5 size-4 shrink-0 text-danger" aria-hidden="true" />
              <span>{error}</span>
            </div>
          )}
        </div>
      </Card>

      {/* Tooling. Browsing works without any of this — only playback and
          downloads go through yt-dlp — so this is setup guidance, never a wall. */}
      {status && (
        <Card title="Playback tools" icon={Terminal}>
          <div className="flex flex-col gap-3">
            {ytdlp?.present ? (
              <>
                <div className="flex items-center gap-2 rounded-control border border-success/30 bg-success/10 px-3 py-2.5 text-sm">
                  <CircleCheck className="size-4 shrink-0 text-success" aria-hidden="true" />
                  <span className="min-w-0">
                    yt-dlp is installed
                    {ytdlp.version ? ` (${ytdlp.version})` : ""} — playback and
                    downloads are ready.
                  </span>
                </div>
                {ytdlp.path && (
                  <p className="truncate font-mono text-xs text-text-faint" title={ytdlp.path}>
                    {ytdlp.path}
                  </p>
                )}
                {!ytdlp.haveFfmpeg && (
                  <div className="flex items-start gap-2 rounded-control border border-border bg-surface px-3 py-2 text-xs text-text-muted">
                    <Info className="mt-0.5 size-3.5 shrink-0 text-text-faint" aria-hidden="true" />
                    <span>
                      ffmpeg isn&rsquo;t installed. Downloads still work, but
                      won&rsquo;t get embedded tags or artwork. Install it the
                      same way as yt-dlp to fix that.
                    </span>
                  </div>
                )}
              </>
            ) : (
              <>
                <div className="flex items-start gap-2 rounded-control border border-accent/30 bg-accent-muted/40 px-3 py-2.5">
                  <Info className="mt-0.5 size-4 shrink-0 text-accent-strong" aria-hidden="true" />
                  <div className="min-w-0 text-sm">
                    <p className="font-medium text-accent-strong">
                      One more step to play tracks
                    </p>
                    <p className="mt-0.5 text-text-muted">
                      Your playlists browse fine without it, but playing and
                      downloading need <span className="text-text">yt-dlp</span>.
                      Install it {install.note}, then reopen this panel.
                    </p>
                  </div>
                </div>
                <InstallCommand command={install.command} />
              </>
            )}
          </div>
        </Card>
      )}

      {/* Downloads */}
      <Card
        title="Downloads"
        icon={FolderOpen}
        actions={
          <div className="flex gap-2">
            <Button variant="ghost" onClick={() => void resetDir()}>
              Reset
            </Button>
            <Button variant="secondary" onClick={() => void pickDir()}>
              <FolderOpen className="size-4" aria-hidden="true" />
              Change
            </Button>
          </div>
        }
      >
        <div className="flex flex-col gap-2">
          <p className="text-sm text-text-muted">
            Downloaded tracks are added to your library, so they play offline and
            seek properly — and keep working if yt-dlp ever breaks.
          </p>
          <p
            className="truncate rounded-control border border-border bg-surface px-3 py-2 font-mono text-xs text-text-muted"
            title={dir ?? undefined}
          >
            {dir ?? "—"}
          </p>
        </div>
      </Card>
    </div>
  );
}

/** A copyable one-line install command. */
function InstallCommand({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="flex items-center gap-2 rounded-control border border-border bg-surface px-3 py-2">
      <Terminal className="size-3.5 shrink-0 text-text-faint" aria-hidden="true" />
      <code className="min-w-0 flex-1 truncate font-mono text-xs text-text">{command}</code>
      <button
        type="button"
        onClick={() => {
          void navigator.clipboard
            .writeText(command)
            .then(() => {
              setCopied(true);
              window.setTimeout(() => setCopied(false), 1500);
            })
            .catch(() => toast.error("Couldn't copy to the clipboard."));
        }}
        className="shrink-0 text-xs text-text-muted transition-colors hover:text-text"
      >
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}
