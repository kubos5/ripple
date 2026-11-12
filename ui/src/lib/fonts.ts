// Interface typefaces. "system" uses the platform font; the others are bundled and loaded on first use.

export const FONTS: { id: string; label: string; sample: string }[] = [
  { id: "system", label: "System", sample: "var(--font-system)" },
  { id: "hanken", label: "Hanken Grotesk", sample: '"Hanken Grotesk Variable"' },
  { id: "plex", label: "IBM Plex Sans", sample: '"IBM Plex Sans Variable"' },
  { id: "atkinson", label: "Atkinson", sample: '"Atkinson Hyperlegible Next Variable"' },
  { id: "serif", label: "Serif", sample: "ui-serif, Georgia, serif" },
];

const loaders: Record<string, () => Promise<unknown>> = {
  hanken: () => import("@fontsource-variable/hanken-grotesk"),
  plex: () => import("@fontsource-variable/ibm-plex-sans"),
  atkinson: () => import("@fontsource-variable/atkinson-hyperlegible-next"),
};

const loaded = new Set<string>();

/** Loads a bundled font's stylesheet once; resolves immediately for system fonts. */
export function loadFont(id: string): Promise<void> {
  const load = loaders[id];
  if (!load || loaded.has(id)) return Promise.resolve();
  loaded.add(id);
  return load().then(() => undefined, () => { loaded.delete(id); });
}

/** Loads every bundled font, so the picker can show each one in its own face. */
export function loadAllFonts(): void {
  for (const id of Object.keys(loaders)) void loadFont(id);
}
