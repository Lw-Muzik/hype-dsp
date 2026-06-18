/** HypeMuzik brand mark: an accent tile with a three-bar equalizer glyph. */
export function Logo({ size = 28 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="none"
      aria-hidden="true"
    >
      <rect width="32" height="32" rx="8" fill="url(#hm-logo-gradient)" />
      <g fill="var(--color-text)">
        <rect x="9" y="13" width="2.6" height="6" rx="1.3" />
        <rect x="14.7" y="9" width="2.6" height="14" rx="1.3" />
        <rect x="20.4" y="11.5" width="2.6" height="9" rx="1.3" />
      </g>
      <defs>
        <linearGradient
          id="hm-logo-gradient"
          x1="0"
          y1="0"
          x2="32"
          y2="32"
          gradientUnits="userSpaceOnUse"
        >
          <stop stopColor="var(--color-accent-strong)" />
          <stop offset="1" stopColor="var(--color-success)" />
        </linearGradient>
      </defs>
    </svg>
  );
}
