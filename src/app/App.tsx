import { useEffect } from "react";
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

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-surface text-text">
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
