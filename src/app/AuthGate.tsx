import { useEffect } from "react";
import type { ReactNode } from "react";
import { AudioLines } from "lucide-react";
import { useAccountStore, toLicenseStatus } from "@/stores/account";
import { useUiStore } from "@/stores/ui";
import { AuthScreen } from "@/components/AuthScreen";
import { TrialEndedScreen } from "@/components/TrialEndedScreen";
import { accountHeartbeat } from "@/lib/ipc";

const REFRESH_MS = 5 * 60 * 1000;
const HEARTBEAT_MS = 60 * 1000;

/**
 * Gates the whole app on the signed-in account + its server-side license.
 * Not authenticated → sign in; authenticated but not allowed (trial over /
 * blocked) → locked screen; otherwise renders the app. Also posts usage
 * heartbeats so the admin dashboard sees who's active.
 */
export function AuthGate({ children }: { children: ReactNode }) {
  const status = useAccountStore((s) => s.status);
  const loading = useAccountStore((s) => s.loading);
  const refresh = useAccountStore((s) => s.refresh);
  const setLicense = useUiStore((s) => s.setLicense);
  const appInfo = useUiStore((s) => s.appInfo);

  // Initial + periodic re-check (a trial can expire / be revoked while running).
  useEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), REFRESH_MS);
    return () => clearInterval(t);
  }, [refresh]);

  // Feed the trial countdown into the existing top banner.
  useEffect(() => {
    const mapped = toLicenseStatus(status);
    if (mapped) setLicense(mapped);
  }, [status, setLicense]);

  // Heartbeats while signed in.
  const authed = status?.authenticated ?? false;
  useEffect(() => {
    if (!authed) return;
    const version = appInfo?.version ?? "dev";
    const platform = detectPlatform();
    void accountHeartbeat(platform, version);
    const t = setInterval(
      () => void accountHeartbeat(platform, version),
      HEARTBEAT_MS,
    );
    return () => clearInterval(t);
  }, [authed, appInfo]);

  if (loading && !status) {
    return (
      <div className="flex h-screen w-screen items-center justify-center bg-surface text-text-muted">
        <AudioLines className="size-7 animate-pulse" aria-hidden="true" />
      </div>
    );
  }

  if (!status?.authenticated) return <AuthScreen />;

  // Block only on an explicit server denial — an unreachable server (license
  // null) grants offline grace rather than locking the user out.
  if (status.license && !status.license.allowed) {
    return <TrialEndedScreen license={status.license} email={status.email} />;
  }

  return <>{children}</>;
}

function detectPlatform(): string {
  if (typeof navigator === "undefined") return "desktop";
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("mac")) return "macos";
  if (ua.includes("win")) return "windows";
  if (ua.includes("linux")) return "linux";
  return "desktop";
}
