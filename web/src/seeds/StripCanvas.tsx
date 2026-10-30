import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type RefObject } from 'react';
import { stripBinText, STRIP_KEYS_NOTE, stripSummary } from '../wording';
import { binStrip, countCodes, keyToBin, nextNotableBin, worstTrial, type StripBin, type StripCode } from './strip';
import { layoutBars, STRIP_HEIGHT } from './stripLayout';

/** CSS variable that colours each class. Colour only adds emphasis: height, the table and the words say the same. */
const COLOUR_VARS: Readonly<Record<StripCode, string>> = {
  v: '--bad',
  x: '--gap',
  u: '--muted',
  i: '--gap',
  e: '--muted',
  '-': '--line',
  p: '--good',
};
const ACTIVE_VAR = '--fg';
const FALLBACK_COLOUR = '#78716c';

function useWidth(ref: RefObject<HTMLDivElement | null>): number {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const read = () => setWidth(Math.floor(el.clientWidth));
    read();
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return width;
}

function colour(style: CSSStyleDeclaration, name: string): string {
  return style.getPropertyValue(name).trim() || FALLBACK_COLOUR;
}

function draw(canvas: HTMLCanvasElement, bins: readonly StripBin[], width: number, active: number | undefined): void {
  const ratio = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.round(width * ratio));
  canvas.height = Math.round(STRIP_HEIGHT * ratio);
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.clearRect(0, 0, width, STRIP_HEIGHT);
  const style = getComputedStyle(canvas);
  const bars = layoutBars(bins, width, STRIP_HEIGHT);
  for (const bar of bars) {
    ctx.fillStyle = colour(style, COLOUR_VARS[bar.code]);
    ctx.fillRect(bar.x, STRIP_HEIGHT - bar.h, bar.w, bar.h);
  }
  const mark = active === undefined ? undefined : bars[active];
  if (mark) {
    ctx.strokeStyle = colour(style, ACTIVE_VAR);
    ctx.lineWidth = 2;
    ctx.strokeRect(mark.x - 1, 1, mark.w + 2, STRIP_HEIGHT - 2);
  }
}

interface Props {
  codes: string;
  /** Open the table at this trial (its sequence number). */
  onOpen: (trial: number) => void;
}

/**
 * The strip, drawn on a canvas: one pixel per trial while there are no more trials than pixels, and a mark per group of
 * trials (showing the worst in it) past that. The canvas is decoration; the control is the slider around it, which
 * keyboard users operate and which reads out the same words the counts table below it gives.
 */
export function StripCanvas({ codes, onOpen }: Props) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const width = useWidth(wrap);
  const bins = useMemo(() => binStrip(codes, width), [codes, width]);
  const [active, setActive] = useState<number>();
  const current = active === undefined ? undefined : bins[active];

  useEffect(() => {
    if (canvas.current && bins.length > 0) draw(canvas.current, bins, width, active);
  }, [bins, width, active]);

  const textOf = (bin: StripBin | undefined): string =>
    bin === undefined ? stripSummary(countCodes(codes)) : stripBinText(bin.start, bin.end, countCodes(codes.slice(bin.start, bin.end)));

  const open = (bin: StripBin | undefined) => bin && onOpen(worstTrial(codes, bin));
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const from = active ?? 0;
    if (e.key === 'Enter' || e.key === ' ') {
      open(bins[from]);
    } else if (e.key === 'n' || e.key === 'p') {
      const next = nextNotableBin(bins, active ?? (e.key === 'n' ? -1 : bins.length), e.key === 'n' ? 1 : -1);
      if (next === undefined) return;
      setActive(next);
    } else {
      const next = keyToBin(e.key, from, bins.length);
      if (next === undefined) return;
      setActive(next);
    }
    e.preventDefault();
  };
  const binAt = (clientX: number): number | undefined => {
    const rect = wrap.current?.getBoundingClientRect();
    if (!rect || bins.length === 0) return undefined;
    return Math.min(bins.length - 1, Math.max(0, Math.floor(((clientX - rect.left) / rect.width) * bins.length)));
  };

  return (
    <>
      <div
        ref={wrap}
        className="strip"
        role="slider"
        tabIndex={0}
        aria-label="Judged trials, in judging order"
        aria-valuemin={0}
        aria-valuemax={Math.max(0, bins.length - 1)}
        aria-valuenow={active ?? 0}
        aria-valuetext={textOf(current)}
        aria-describedby="strip-keys"
        style={{ height: STRIP_HEIGHT }}
        onKeyDown={onKeyDown}
        onMouseMove={(e) => setActive(binAt(e.clientX))}
        onMouseLeave={() => setActive(undefined)}
        onClick={(e) => open(bins[binAt(e.clientX) ?? -1])}
      >
        <canvas ref={canvas} aria-hidden="true" height={STRIP_HEIGHT} style={{ width: '100%', height: STRIP_HEIGHT }} />
      </div>
      <p className="muted strip-status" role="status">
        {textOf(current)}
      </p>
      <p id="strip-keys" className="muted strip-keys">
        {STRIP_KEYS_NOTE}
      </p>
    </>
  );
}
