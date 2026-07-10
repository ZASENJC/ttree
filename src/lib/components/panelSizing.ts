export const MIN_SOURCE_HEIGHT = 88;
export const MIN_RESULT_HEIGHT = 120;
export const PANEL_CHROME_HEIGHT = 128;

export function maxSourceHeight(panelHeight: number): number {
  return Math.max(
    MIN_SOURCE_HEIGHT,
    panelHeight - MIN_RESULT_HEIGHT - PANEL_CHROME_HEIGHT,
  );
}

export function clampSourceHeight(
  sourceHeight: number,
  panelHeight: number,
): number {
  return Math.min(
    Math.max(sourceHeight, MIN_SOURCE_HEIGHT),
    maxSourceHeight(panelHeight),
  );
}
