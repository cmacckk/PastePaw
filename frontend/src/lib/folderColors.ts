/**
 * The colours a folder can be given, and the colour it falls back to.
 *
 * A stored colour wins. Without one a colour is derived from the name, so a folder keeps
 * the same dot between runs even though nothing was saved for it.
 */
export const FOLDER_DOT_COLORS = [
  '#ef4444',
  '#f97316',
  '#eab308',
  '#22c55e',
  '#14b8a6',
  '#06b6d4',
  '#3b82f6',
  '#8b5cf6',
  '#ec4899',
  '#64748b',
];

export function folderDotColor(name: string, color?: string | null) {
  if (color) {
    return color;
  }
  let hash = 0;
  for (let i = 0; i < name.length; i += 1) {
    hash = name.charCodeAt(i) + ((hash << 5) - hash);
  }
  return FOLDER_DOT_COLORS[Math.abs(hash) % FOLDER_DOT_COLORS.length];
}
