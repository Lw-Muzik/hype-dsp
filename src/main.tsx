import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "@/app/App";
import { SessionGate } from "@/app/SessionGate";
import { OnboardingGate } from "@/app/OnboardingGate";
import { IntroSplash } from "@/components/IntroSplash";
import { Providers } from "@/app/providers";
import "@/styles/index.css";

const rootEl = document.getElementById("root");
if (!rootEl) {
  throw new Error("Root element #root not found in index.html");
}
const root = createRoot(rootEl);

root.render(
  <StrictMode>
    <Providers>
      <OnboardingGate>
        <SessionGate>
          <App />
        </SessionGate>
      </OnboardingGate>
      <IntroSplash />
    </Providers>
  </StrictMode>
);
