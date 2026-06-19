import logoUrl from "@/assets/logo.png";

/** HypeMuzik brand mark: gold disc + green note, floating on the dark UI. */
export function Logo({ size = 28 }: { size?: number }) {
  return (
    <img
      src={logoUrl}
      alt=""
      aria-hidden="true"
      draggable={false}
      style={{ height: size, width: "auto" }}
      className="block select-none"
    />
  );
}
