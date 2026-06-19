import { Sidebar } from "@/components/Sidebar";
import { TopBar } from "@/components/TopBar";
import { TrialBanner } from "@/components/TrialBanner";
import { Toaster } from "@/components/Toaster";
import { NowPlayingBar } from "@/components/NowPlayingBar";
import { RightSidebar } from "@/components/RightSidebar";
import { ResizeHandle } from "@/components/ResizeHandle";
import { ScrollingWaveform } from "@/features/player/ScrollingWaveform";
import { Router } from "@/app/router";

/** The application shell: sidebar + top bar + the active view + now-playing. */
export function App() {
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
        <ScrollingWaveform />
        <NowPlayingBar />
      </div>
      <Toaster />
    </div>
  );
}
