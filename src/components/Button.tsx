import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cn } from "@/lib/cn";

type Variant = "primary" | "secondary" | "ghost";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  children: ReactNode;
}

const variants: Record<Variant, string> = {
  // text-on-accent, not text-text: the fill is a mid-tone amber, and light text
  // on it measures 1.58:1. This token flips with the theme so the pairing holds.
  primary: "bg-accent text-on-accent hover:bg-accent-strong",
  secondary:
    "border border-border bg-surface-raised text-text hover:bg-surface-overlay",
  ghost: "text-text-muted hover:bg-surface-raised hover:text-text",
};

/** The app's standard button. */
export function Button({
  variant = "secondary",
  className,
  children,
  type = "button",
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={cn(
        "inline-flex items-center justify-center gap-2 rounded-control px-3.5 py-2 text-sm font-medium transition-colors",
        "disabled:pointer-events-none disabled:opacity-50",
        variants[variant],
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}
