import { useEffect } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import ThemeBackdrop from "@/features/theme/ThemeBackdrop";
import { Sidebar } from "@/components/Sidebar";
import { TopBar } from "@/components/TopBar";
import { TrialBanner } from "@/components/TrialBanner";
import { Toaster } from "@/components/Toaster";
import { NowPlayingBar } from "@/components/NowPlayingBar";
import { RightSidebar } from "@/components/RightSidebar";
import { ResizeHandle } from "@/components/ResizeHandle";
import { Router } from "@/app/router";
import { useSystemEqStore } from "@/stores/systemEq";
import { useMusicLibraryStore } from "@/stores/musicLibrary";
import {
  linkDiscoverStart,
  onPairedOnline,
  onRemoteConnected,
} from "@/lib/ipc";

/** The application shell: sidebar + top bar + the active view + now-playing. */
export function App() {
  // If the user left system-wide EQ on last session, re-engage it on launch.
  const resumeSystemEq = useSystemEqStore((s) => s.resume);
  useEffect(() => {
    void resumeSystemEq();
  }, [resumeSystemEq]);

  // Re-check the local library when the app regains focus: a drive may have been
  // plugged in or ejected while we were away, so tracks should appear/disappear.
  // The probe is cheap and only reloads when availability actually changed.
  const revalidateLocal = useMusicLibraryStore((s) => s.revalidateLocal);
  useEffect(() => {
    const onFocus = () => revalidateLocal();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [revalidateLocal]);

  // Keep the phone library in sync automatically. Run LAN discovery for the
  // whole session (not just while the Phone screen is open) and refresh a
  // phone's tracks the moment it becomes reachable — a paired phone coming
  // online, or opened after launch, now appears without a relaunch. The reload
  // is debounced so repeated mDNS announcements coalesce into one sync, and it
  // refreshes in place (no empty flash, works even off the Library tab).
  useEffect(() => {
    void linkDiscoverStart().catch(() => {});
    let cancelled = false;
    let timer: number | undefined;
    const unlisteners: UnlistenFn[] = [];
    const scheduleSync = () => {
      if (timer) window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        useMusicLibraryStore.getState().syncPhone();
      }, 1200);
    };
    const register = (p: Promise<UnlistenFn>) => {
      void p
        .then((fn) => (cancelled ? fn() : unlisteners.push(fn)))
        .catch(() => {});
    };
    register(onPairedOnline(scheduleSync));
    register(onRemoteConnected(scheduleSync));
    return () => {
      cancelled = true;
      if (timer) window.clearTimeout(timer);
      unlisteners.forEach((fn) => fn());
    };
  }, []);

  return (
    <div className="relative isolate flex h-screen w-screen overflow-hidden bg-surface text-text">
      <ThemeBackdrop />
      <Sidebar />
      <ResizeHandle side="left" />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar />
        <TrialBanner />
        <div className="flex min-h-0 flex-1">
          <main className="min-h-0 flex-1 overflow-y-auto p-6">
            <Router />
          </main>
          <ResizeHandle side="right" />
          <RightSidebar />
        </div>
        <NowPlayingBar />
      </div>
      <Toaster />
    </div>
  );
}
