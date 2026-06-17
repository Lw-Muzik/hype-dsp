import { Sidebar } from "@/components/Sidebar";
import { TopBar } from "@/components/TopBar";
import { TrialBanner } from "@/components/TrialBanner";
import { Router } from "@/app/router";

/** The application shell: sidebar + top bar + the active view. */
export function App() {
  return (
    <div className="flex h-screen w-screen overflow-hidden bg-surface text-text">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar />
        <TrialBanner />
        <main className="flex-1 overflow-y-auto p-6">
          <Router />
        </main>
      </div>
    </div>
  );
}
