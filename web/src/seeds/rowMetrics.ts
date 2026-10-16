/**
 * Height of every row of the trial table, defined once. A fixed row height is what makes windowing a sum: the table's
 * scroll height is exactly the number of rows times this, and a test in a real browser checks that it still is.
 */
export const ROW_HEIGHT = 40;
/** Vertical padding (2 x 8px) plus the 1px row border, set in seeds.css. */
export const ROW_CHROME = 17;
/** Every cell's content sits in a block of exactly this height with overflow hidden, so no row can grow. */
export const CLIP_HEIGHT = ROW_HEIGHT - ROW_CHROME;
export const VIEWPORT_HEIGHT = 480;
export const OVERSCAN = 8;
/** Rows above the first data row: the column headers. */
export const HEADER_ROWS = 1;
/** Rows from the end of the loaded list at which the next page is asked for. */
export const NEAR_END_ROWS = 20;
