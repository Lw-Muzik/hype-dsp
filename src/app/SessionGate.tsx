import { useEffect, useRef } from "react";
import type { ReactNode } from "react";
import { AudioLines } from "lucide-react";
import { useAccountStore } from "@/stores/account";
import { useEntitlementStore } from "@/stores/entitlement";
import { useUiStore } from "@/stores/ui";
import { accountHeartbeat } from "@/lib/ipc";

const REFRESH_MS = 5 * 60 * 1000;
const HEARTBEAT_MS = 60 * 1000;

/**
 * Frictionless session bootstrap — replaces the old sign-in wall.
 *
 * Core playback, library, basic EQ, Stations, visualizers, and YouTube Music
 * are free forever, so this gate never blocks: it silently provisions an
 * anonymous session (keyed off the keychain install id — see
 * `commands::account::account_bootstrap_anonymous`) on first launch, keeps
 * heartbeats and the Supporter-entitlement check current, and always renders
 * `children`. It NEVER returns a sign-in screen or a "trial ended" screen —
 * gating for Advanced DSP / stems / cloud happens per-feature via
 * `<PremiumGate>` / `requirePremium`, not here. Offline on first launch still
 * opens the app (core needs no network); the bootstrap retries on the next
 * focus or periodic refresh.
 */
export function SessionGate({ children }: { children: ReactNode }) {
  const status = useAccountStore((s) => s.status);
  const loading = useAccountStore((s) => s.loading);
  const refresh = useAccountStore((s) => s.refresh);
  const bootstrapAnonymous = useAccountStore((s) => s.bootstrapAnonymous);
  const refreshEntitlement = useEntitlementStore((s) => s.refresh);
  const appInfo = useUiStore((s) => s.appInfo);

  // Initial + periodic re-check (a claimed email / Supporter status can
  // change server-side while running).
  useEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), REFRESH_MS);
    return () => clearInterval(t);
  }, [refresh]);

  // Ensure a session always exists. Fires whenever we learn there isn't
  // one — first launch (no local tokens), a dead refresh token that got
  // cleared, or right after the user signs out — so the app is never stuck
  // sessionless. Idempotent server-side (same install id → same user), and
  // guarded here so an offline stretch doesn't fire this on every re-render.
  const bootstrapping = useRef(false);
  useEffect(() => {
    if (!status || status.authenticated || bootstrapping.current) return;
    bootstrapping.current = true;
    void bootstrapAnonymous().finally(() => {
      bootstrapping.current = false;
    });
  }, [status, bootstrapAnonymous]);

  // Entitlements follow the session: fetch as soon as we're authenticated,
  // and on the periodic/focus refreshes below.
  useEffect(() => {
    if (status?.authenticated) void refreshEntitlement();
  }, [status?.authenticated, refreshEntitlement]);

  // Re-check everything whenever the app regains focus / becomes visible.
  // The interval above is suspended while the machine sleeps, so without this
  // a claimed-email / Supporter change (or a first-launch offline bootstrap)
  // could sit stale until the next 5-min tick. Throttled so rapid focus
  // changes don't spam the server; the single-flight token refresh makes
  // frequent re-checks safe.
  const lastActiveRefresh = useRef(Date.now());
  useEffect(() => {
    const onActive = () => {
      if (document.visibilityState !== "visible") return;
      const now = Date.now();
      if (now - lastActiveRefresh.current < 30_000) return;
      lastActiveRefresh.current = now;
      void refresh();
      void refreshEntitlement();
    };
    window.addEventListener("focus", onActive);
    document.addEventListener("visibilitychange", onActive);
    return () => {
      window.removeEventListener("focus", onActive);
      document.removeEventListener("visibilitychange", onActive);
    };
  }, [refresh, refreshEntitlement]);

  // Heartbeats while a session exists (anonymous sessions are "authenticated"
  // too, so usage is counted from the very first launch).
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

  // Only ever a brief spinner for the very first bootstrap — never a wall.
  if (loading && !status) {
    return (
      <div className="flex h-screen w-screen items-center justify-center bg-surface text-text-muted">
        <AudioLines className="size-7 animate-pulse" aria-hidden="true" />
      </div>
    );
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
