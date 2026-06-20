import { useEffect, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  AudioLines,
  Globe,
  SlidersVertical,
  Sparkles,
  Waves,
  type LucideIcon,
} from "lucide-react";
import { Button } from "@/components/Button";
import { cn } from "@/lib/cn";

interface Slide {
  icon: LucideIcon;
  title: string;
  body: string;
}

const SLIDES: Slide[] = [
  {
    icon: AudioLines,
    title: "Welcome to HypeMuzik",
    body: "Studio-grade sound for everything you play — shaped exactly to your ears and your gear.",
  },
  {
    icon: SlidersVertical,
    title: "Shape your sound",
    body: "A 31-band equalizer, 37 AutoEq headphone profiles, plus bass, 3D surround and room reverb. Make every track yours.",
  },
  {
    icon: Waves,
    title: "Enhance every app",
    body: "System-wide EQ runs underneath everything — streaming, games, calls — so all of your audio sounds better, not just HypeMuzik.",
  },
  {
    icon: Sparkles,
    title: "See the music",
    body: "MilkDrop visuals and beat-reactive scenes that move in time with whatever's playing.",
  },
  {
    icon: Globe,
    title: "Your music, everywhere",
    body: "Stream your phone's library and cloud drives — on the same Wi‑Fi, or across the internet, securely peer-to-peer.",
  },
];

/** First-launch presentation of what HypeMuzik offers, shown once. */
export function Onboarding({ onComplete }: { onComplete: () => void }) {
  const [i, setI] = useState(0);
  const last = i === SLIDES.length - 1;

  const next = () => (last ? onComplete() : setI((c) => c + 1));
  const back = () => setI((c) => Math.max(0, c - 1));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowRight" || e.key === "Enter") next();
      else if (e.key === "ArrowLeft") back();
      else if (e.key === "Escape") onComplete();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [i, last]);

  return (
    <div className="relative flex h-screen w-screen flex-col overflow-hidden bg-surface text-text">
      {/* Brand-coloured ambient backdrop. */}
      <div aria-hidden className="pointer-events-none absolute inset-0">
        <div className="absolute -top-1/3 left-1/2 size-[55vmax] -translate-x-1/2 rounded-full bg-accent/15 blur-[130px]" />
        <div className="absolute -bottom-1/3 left-1/4 size-[40vmax] rounded-full bg-success/10 blur-[130px]" />
      </div>

      <header className="relative z-10 flex items-center justify-between px-7 py-5">
        <div className="flex items-center gap-2">
          <div className="grid size-8 place-items-center rounded-lg bg-gradient-to-br from-accent to-success text-surface">
            <AudioLines className="size-4" aria-hidden="true" />
          </div>
          <span className="text-sm font-semibold">HypeMuzik</span>
        </div>
        <button
          type="button"
          onClick={onComplete}
          className="text-xs text-text-muted transition-colors hover:text-text"
        >
          Skip
        </button>
      </header>

      <main className="relative z-10 flex flex-1 items-center justify-center px-6">
        <div className="relative h-72 w-full max-w-md">
          {SLIDES.map((s, idx) => {
            const Icon = s.icon;
            return (
              <div
                key={idx}
                className={cn(
                  "absolute inset-0 flex flex-col items-center text-center transition-all duration-500 ease-out",
                  idx === i
                    ? "translate-x-0 opacity-100"
                    : cn(
                        "pointer-events-none opacity-0",
                        idx < i ? "-translate-x-8" : "translate-x-8",
                      ),
                )}
              >
                <div className="mb-7 grid size-20 place-items-center rounded-2xl bg-gradient-to-br from-accent to-success text-surface shadow-lg shadow-accent/20">
                  <Icon className="size-9" aria-hidden="true" />
                </div>
                <h1 className="text-2xl font-semibold tracking-tight">
                  {s.title}
                </h1>
                <p className="mt-3 max-w-sm text-text-muted">{s.body}</p>
              </div>
            );
          })}
        </div>
      </main>

      <footer className="relative z-10 flex items-center justify-between px-7 py-7">
        <div className="flex w-24">
          {i > 0 && (
            <Button variant="ghost" onClick={back}>
              <ArrowLeft className="size-4" aria-hidden="true" />
              Back
            </Button>
          )}
        </div>

        <div className="flex items-center gap-2">
          {SLIDES.map((_, idx) => (
            <button
              key={idx}
              type="button"
              onClick={() => setI(idx)}
              aria-label={`Go to slide ${idx + 1}`}
              className={cn(
                "h-1.5 rounded-full transition-all",
                idx === i ? "w-6 bg-accent" : "w-1.5 bg-border hover:bg-text-faint",
              )}
            />
          ))}
        </div>

        <div className="flex w-24 justify-end">
          <Button variant="primary" onClick={next}>
            {last ? "Get started" : "Next"}
            <ArrowRight className="size-4" aria-hidden="true" />
          </Button>
        </div>
      </footer>
    </div>
  );
}
