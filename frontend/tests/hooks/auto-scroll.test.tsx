import { afterEach, beforeEach, expect, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { useAutoScroll } from '../../src/hooks/useAutoScroll';

class ScrollContainer extends EventTarget {
  scrollHeight = 1000;
  clientHeight = 300;
  private top = 700;
  get scrollTop() { return this.top; }
  set scrollTop(value: number) { this.top = Math.max(0, Math.min(value, this.scrollHeight - this.clientHeight)); }
  scrollTo(top: number) {
    this.scrollTop = top;
    this.dispatchEvent(new Event('scroll'));
  }
}

let container: ScrollContainer;
let renderer: ReactTestRenderer | undefined;
let resize: () => void;
let frames: Map<number, FrameRequestCallback>;
let nextFrame = 0;
const originals = {
  resize: globalThis.ResizeObserver,
  request: globalThis.requestAnimationFrame,
  cancel: globalThis.cancelAnimationFrame,
};

function View({ count, disabled = false }: { count: number; disabled?: boolean }) {
  useAutoScroll({
    scrollRef: { current: container as unknown as HTMLDivElement },
    contentRef: { current: {} as HTMLDivElement },
    segments: Array.from({ length: count }, (_, id) => ({ id })),
    isRecording: true,
    isPaused: false,
    disableAutoScroll: disabled,
  });
  return null;
}

async function show(count: number, disabled = false) {
  await act(async () => {
    if (renderer) renderer.update(<View count={count} disabled={disabled} />);
    else renderer = create(<View count={count} disabled={disabled} />);
  });
}

beforeEach(() => {
  container = new ScrollContainer();
  frames = new Map();
  globalThis.ResizeObserver = class {
    constructor(callback: () => void) { resize = callback; }
    observe() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  globalThis.requestAnimationFrame = callback => { frames.set(++nextFrame, callback); return nextFrame; };
  globalThis.cancelAnimationFrame = id => { frames.delete(id); };
});

afterEach(async () => {
  if (renderer) await act(async () => renderer!.unmount());
  renderer = undefined;
  globalThis.ResizeObserver = originals.resize;
  globalThis.requestAnimationFrame = originals.request;
  globalThis.cancelAnimationFrame = originals.cancel;
});

test('follows a long new phrase even when growth puts the old position far from bottom', async () => {
  await show(1);
  container.scrollHeight += 500;
  await show(2);
  expect(container.scrollTop).toBe(1200);
});

test('preserves manual scroll immediately, then resumes after returning to bottom', async () => {
  await show(1);
  await act(async () => container.scrollTo(400));
  container.scrollHeight += 500;
  await show(2);
  expect(container.scrollTop).toBe(400);
  await act(async () => container.scrollTo(1200));
  container.scrollHeight += 200;
  await show(3);
  expect(container.scrollTop).toBe(1400);
});

test('follows streaming height changes and cancels following if user scrolls before the frame', async () => {
  await show(1);
  container.scrollHeight += 200;
  resize();
  await act(async () => {
    for (const callback of frames.values()) callback(0);
    frames.clear();
  });
  expect(container.scrollTop).toBe(900);
  container.scrollHeight += 200;
  resize();
  await act(async () => container.scrollTo(500));
  await act(async () => {
    for (const callback of frames.values()) callback(0);
    frames.clear();
  });
  expect(container.scrollTop).toBe(500);
});

test('leaves saved meeting scroll untouched when auto-scroll is disabled', async () => {
  container.scrollTop = 200;
  await show(1, true);
  container.scrollHeight += 500;
  await show(2, true);
  expect(container.scrollTop).toBe(200);
});
