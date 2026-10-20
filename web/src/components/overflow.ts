/** Browsers round `scrollWidth` and `clientWidth` to whole pixels, so a difference under this is not an overflow. */
const SUBPIXEL_PX = 1;

/** Whether content of `scrollWidth` does not fit a box of `clientWidth`, so the box can be scrolled sideways. */
export function isOverflowing(scrollWidth: number, clientWidth: number): boolean {
  return scrollWidth - clientWidth > SUBPIXEL_PX;
}
