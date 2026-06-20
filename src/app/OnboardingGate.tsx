import { useState } from "react";
import type { ReactNode } from "react";
import { Onboarding } from "@/components/Onboarding";

const ONBOARDED_KEY = "hm_onboarded";

/** Shows the first-launch presentation once, then renders the app (auth gate). */
export function OnboardingGate({ children }: { children: ReactNode }) {
  const [done, setDone] = useState(() => {
    try {
      return localStorage.getItem(ONBOARDED_KEY) === "1";
    } catch {
      return true; // storage unavailable — don't trap the user on onboarding
    }
  });

  if (!done) {
    return (
      <Onboarding
        onComplete={() => {
          try {
            localStorage.setItem(ONBOARDED_KEY, "1");
          } catch {
            /* ignore */
          }
          setDone(true);
        }}
      />
    );
  }

  return <>{children}</>;
}
