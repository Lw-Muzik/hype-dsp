import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "@/app/App";
import { SessionGate } from "@/app/SessionGate";
import { OnboardingGate } from "@/app/OnboardingGate";
import { Providers } from "@/app/providers";
import { startMainWindowSync, startSettingsWindowSync } from "@/app/windowSyncSetup";
import { SettingsWindow } from "@/features/settings/window/SettingsWindow";
import { isSettingsWindow } from "@/lib/windowLabel";
import "@/styles/index.css";

const rootEl = document.getElementById("root");
if (!rootEl) {
  throw new Error("Root element #root not found in index.html");
}
const root = createRoot(rootEl);

/** Settings starts from the main window's live state, but never waits on it forever. */
const SETTINGS_SYNC_TIMEOUT_MS = 1500;

if (isSettingsWindow) {
  // The Settings window runs none of the main window's global effects
  // (Providers, gates, App): they would double-publish Hype Connect state,
  // re-drain open-files, re-resume system EQ and so on.
  await Promise.race([
    startSettingsWindowSync().catch(() => {}),
    new Promise<void>((resolve) => setTimeout(resolve, SETTINGS_SYNC_TIMEOUT_MS)),
  ]);
  root.render(
    <StrictMode>
      <SettingsWindow />
    </StrictMode>
  );
} else {
  startMainWindowSync();
  root.render(
    <StrictMode>
      <Providers>
        <OnboardingGate>
          <SessionGate>
            <App />
          </SessionGate>
        </OnboardingGate>
      </Providers>
    </StrictMode>
  );
}
