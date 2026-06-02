export function humanizeMachineLabel(
  value: string | null | undefined,
  fallback = "-",
): string {
  const cleaned = String(value || "")
    .trim()
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!cleaned) return fallback;
  return cleaned
    .split(" ")
    .map((part) => {
      if (!part) return part;
      if (part.length > 1 && /^[A-Z0-9]+$/.test(part)) return part;
      return `${part.charAt(0).toUpperCase()}${part.slice(1)}`;
    })
    .join(" ");
}

export function humanizeStatusLabel(
  value: string | null | undefined,
  fallback = "-",
): string {
  return humanizeMachineLabel(value, fallback);
}
