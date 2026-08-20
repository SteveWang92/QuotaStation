/**
 * The chart palette, as roles rather than colours.
 *
 * The slots are assigned in a fixed order and never cycled: a series keeps its colour when
 * the range changes and other series come and go, so a reader who learned that output is
 * orange is not misled by the next range. The hexes themselves live in `styles.css`, where
 * they were chosen for the dark chart surface and checked for colour-vision separation as a
 * set; anything past the fourth slot folds into `SERIES_REST` rather than inventing a fifth
 * hue that nobody could tell from the ones already on screen.
 */
export const SERIES_SLOTS = [
  "var(--series-1)",
  "var(--series-2)",
  "var(--series-3)",
  "var(--series-4)",
] as const;

/** The de-emphasis grey the folded tail of a long list is drawn in. */
export const SERIES_REST = "var(--series-rest)";

/** How many named series a chart shows before the rest become "Other". */
export const SERIES_LIMIT = SERIES_SLOTS.length;
