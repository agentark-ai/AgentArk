export function cssToken(name: string): string {
  return `var(${name})`;
}

export function resolveCssToken(name: string): string {
  if (typeof window === "undefined") return cssToken(name);
  const value = window
    .getComputedStyle(window.document.documentElement)
    .getPropertyValue(name)
    .trim();
  return value || cssToken(name);
}
