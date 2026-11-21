/** Input token accepted by `cx()` for static CSS class composition. */
export type ClassInput = string | false | null | undefined | readonly ClassInput[];

/**
 * Joins CSS module and utility class names, skipping falsy values.
 * Zero runtime CSS merging: produces a single space-separated `class` string.
 */
export function cx(...inputs: readonly ClassInput[]): string {
  const parts: string[] = [];
  const visit = (item: ClassInput): void => {
    if (!item) return;
    if (typeof item === "string") {
      const trimmed = item.trim();
      if (trimmed.length > 0) parts.push(trimmed);
      return;
    }
    for (let i = 0; i < item.length; i++) {
      visit(item[i]);
    }
  };
  for (let i = 0; i < inputs.length; i++) {
    visit(inputs[i]);
  }
  return parts.join(" ");
}
