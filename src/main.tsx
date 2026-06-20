import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "@/app/App";
import { AuthGate } from "@/app/AuthGate";
import { OnboardingGate } from "@/app/OnboardingGate";
import { Providers } from "@/app/providers";
import "@/styles/index.css";

const rootEl = document.getElementById("root");
if (!rootEl) {
  throw new Error("Root element #root not found in index.html");
}

createRoot(rootEl).render(
  <StrictMode>
    <Providers>
      <OnboardingGate>
        <AuthGate>
          <App />
        </AuthGate>
      </OnboardingGate>
    </Providers>
  </StrictMode>,
);
