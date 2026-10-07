'use client';

import { useLayoutEffect, useRef, useState, type PointerEvent, type KeyboardEvent } from 'react';
import { LiveAudioStatus } from './LiveAudioStatus';
import { useI18n } from '@/lib/i18n';

type Point = { x: number; y: number };
const MARGIN = 12;

export function DraggableAudioStatus({ devices, recording, disabled = false }: {
  devices?: { micDevice: string | null; systemDevice: string | null };
  recording: boolean;
  disabled?: boolean;
}) {
  const areaRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const positionRef = useRef<Point | null>(null);
  const dragRef = useRef<{ pointerId: number; origin: Point; start: Point } | null>(null);
  const [position, setPosition] = useState<Point | null>(null);
  const [dragging, setDragging] = useState(false);
  const { t } = useI18n();

  const place = (point?: Point) => {
    const area = areaRef.current;
    const panel = panelRef.current;
    if (!area || !panel) return;
    const maxX = Math.max(MARGIN, area.clientWidth - panel.offsetWidth - MARGIN);
    const maxY = Math.max(MARGIN, area.clientHeight - panel.offsetHeight - MARGIN);
    const desired = point ?? positionRef.current ?? {
      x: (area.clientWidth - panel.offsetWidth) / 2,
      y: area.clientHeight - panel.offsetHeight - 112,
    };
    const next = {
      x: Math.min(maxX, Math.max(MARGIN, desired.x)),
      y: Math.min(maxY, Math.max(MARGIN, desired.y)),
    };
    positionRef.current = next;
    setPosition(previous => previous?.x === next.x && previous.y === next.y ? previous : next);
  };

  useLayoutEffect(() => {
    place();
    const observer = new ResizeObserver(() => place());
    if (areaRef.current) observer.observe(areaRef.current);
    if (panelRef.current) observer.observe(panelRef.current);
    return () => observer.disconnect();
  }, []);

  const startDrag = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || (event.target as HTMLElement).closest('button, a, input, select')) return;
    event.preventDefault();
    event.currentTarget.focus({ preventScroll: true });
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = {
      pointerId: event.pointerId,
      origin: positionRef.current ?? { x: 0, y: 0 },
      start: { x: event.clientX, y: event.clientY },
    };
    setDragging(true);
  };

  const moveDrag = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    place({
      x: drag.origin.x + event.clientX - drag.start.x,
      y: drag.origin.y + event.clientY - drag.start.y,
    });
  };

  const endDrag = (event: PointerEvent<HTMLDivElement>) => {
    if (dragRef.current?.pointerId !== event.pointerId) return;
    dragRef.current = null;
    setDragging(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  };

  const moveWithKeyboard = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    const delta: Record<string, Point> = {
      ArrowLeft: { x: -16, y: 0 }, ArrowRight: { x: 16, y: 0 },
      ArrowUp: { x: 0, y: -16 }, ArrowDown: { x: 0, y: 16 },
    };
    if (!delta[event.key]) return;
    event.preventDefault();
    const current = positionRef.current ?? { x: 0, y: 0 };
    place({ x: current.x + delta[event.key].x, y: current.y + delta[event.key].y });
  };

  return <div ref={areaRef} className="pointer-events-none absolute inset-0 z-20 overflow-hidden">
    <div ref={panelRef}
      className="pointer-events-auto absolute w-80 max-w-[calc(100%_-_24px)] max-h-[calc(100%_-_24px)] overflow-y-auto rounded-md shadow-lg"
      style={{ left: position?.x ?? 0, top: position?.y ?? 0, visibility: position ? 'visible' : 'hidden' }}
    >
      <LiveAudioStatus recording={recording} alwaysVisible disabled={disabled} devices={devices} dragHandleProps={{
        tabIndex: 0,
        'aria-label': t('Move audio panel. Drag the header or use arrow keys.'),
        title: t('Drag to move'),
        className: `touch-none select-none rounded focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 ${dragging ? 'cursor-grabbing' : 'cursor-grab'}`,
        onPointerDown: startDrag,
        onPointerMove: moveDrag,
        onPointerUp: endDrag,
        onPointerCancel: endDrag,
        onLostPointerCapture: endDrag,
        onKeyDown: moveWithKeyboard,
      }} />
    </div>
  </div>;
}
