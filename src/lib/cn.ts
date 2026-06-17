/**
 * Join class names, dropping falsy entries. A dependency-free `clsx`.
 */
export function cn(
  ...parts: Array<string | false | null | undefined>
): string {
  return parts.filter(Boolean).join(" ");
}
