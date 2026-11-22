import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  type KeyboardEvent,
  type PointerEvent,
  type Ref,
} from 'react';
import type { DragSnapHint } from './workbenchDragPolicy';

export interface ResizableSeparatorDragResult {
  value: number;
  snapHint?: DragSnapHint;
}

interface ResizableSeparatorProps {
  ref?: Ref<HTMLDivElement>;
  label: string;
  value: number;
  min: number;
  max: number;
  defaultValue: number;
  disabled?: boolean;
  /** Direction of the pane whose width this separator owns. */
  direction?: 1 | -1;
  onChange: (value: number) => void;
  onCommit?: (value: number) => void;
  onDragStart?: () => void;
  onDragEnd?: () => void;
  onDragChange?: (value: number) => void;
  /** Optional raw-width drag handler supporting snap/collapse hysteresis. */
  onDragRaw?: (rawValue: number) => ResizableSeparatorDragResult;
  onDoubleClick?: () => void;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}

function assignRef<T>(target: Ref<T> | undefined, value: T | null) {
  if (!target) return;
  if (typeof target === 'function') target(value);
  else target.current = value;
}

export function ResizableSeparator({
  ref,
  label,
  value,
  min,
  max,
  defaultValue,
  direction = 1,
  disabled = false,
  onChange,
  onCommit,
  onDragStart,
  onDragEnd,
  onDragChange,
  onDragRaw,
  onDoubleClick,
}: ResizableSeparatorProps) {
  const separatorRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<{
    pointerId: number;
    startX: number;
    startValue: number;
    lastX: number;
  } | null>(null);
  const dragValueRef = useRef(value);
  const frameRef = useRef<number | null>(null);
  const latestRef = useRef({ min, max, direction, onChange, onDragChange, onDragRaw });

  const setDraggingClass = (active: boolean) => {
    document.body.classList.toggle('is-resizing', active);
    separatorRef.current?.classList.toggle('is-dragging', active);
    if (!active) separatorRef.current?.removeAttribute('data-snap-hint');
  };

  useEffect(
    () => () => {
      if (frameRef.current !== null) window.cancelAnimationFrame(frameRef.current);
      if (dragRef.current !== null) {
        dragRef.current = null;
        document.body.classList.remove('is-resizing');
      }
    },
    [],
  );

  const applyLatest = useCallback(() => {
    const drag = dragRef.current;
    if (!drag) return;
    const { min, max, direction, onChange, onDragChange, onDragRaw } = latestRef.current;
    const raw = Math.round(drag.startValue + direction * (drag.lastX - drag.startX));

    if (onDragRaw) {
      const resolved = onDragRaw(raw);
      if (resolved.snapHint) {
        separatorRef.current?.setAttribute('data-snap-hint', resolved.snapHint);
      } else {
        separatorRef.current?.removeAttribute('data-snap-hint');
      }
      if (resolved.value === dragValueRef.current) return;
      dragValueRef.current = resolved.value;
      separatorRef.current?.setAttribute('aria-valuenow', String(resolved.value));
      return;
    }

    const next = clamp(raw, min, max);
    if (next === dragValueRef.current) return;
    dragValueRef.current = next;
    separatorRef.current?.setAttribute('aria-valuenow', String(next));
    (onDragChange ?? onChange)(next);
  }, []);

  const schedule = useCallback(() => {
    if (frameRef.current !== null || !dragRef.current) return;
    frameRef.current = window.requestAnimationFrame(() => {
      frameRef.current = null;
      applyLatest();
    });
  }, [applyLatest]);

  // Parent renders refresh handlers without cancelling or postponing a frame.
  // Only a real bounds change can require a frame without new pointer input.
  useLayoutEffect(() => {
    const previous = latestRef.current;
    latestRef.current = { min, max, direction, onChange, onDragChange, onDragRaw };
    if (previous.min !== min || previous.max !== max || previous.direction !== direction) {
      schedule();
    }
  }, [min, max, direction, onChange, onDragChange, onDragRaw, schedule]);

  const finishDrag = () => {
    if (!dragRef.current) return;
    if (frameRef.current !== null) window.cancelAnimationFrame(frameRef.current);
    frameRef.current = null;
    applyLatest();
    dragRef.current = null;
    setDraggingClass(false);
    onCommit?.(dragValueRef.current);
    onDragEnd?.();
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (disabled || event.button !== 0 || dragRef.current) return;
    event.preventDefault();
    dragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startValue: value,
      lastX: event.clientX,
    };
    dragValueRef.current = value;
    setDraggingClass(true);
    onDragStart?.();
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    drag.lastX = event.clientX;
    schedule();
  };

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (dragRef.current?.pointerId !== event.pointerId) return;
    finishDrag();
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (disabled) return;
    let next: number | null = null;
    const step = event.shiftKey ? 32 : 8;
    if (event.key === 'ArrowLeft') next = value - direction * step;
    if (event.key === 'ArrowRight') next = value + direction * step;
    if (event.key === 'Home') next = min;
    if (event.key === 'End') next = max;
    if (event.key === 'Enter') next = defaultValue;
    if (next === null) return;
    event.preventDefault();
    const clamped = clamp(next, min, max);
    dragValueRef.current = clamped;
    onChange(clamped);
    onCommit?.(clamped);
  };

  return (
    <div
      ref={(node) => {
        separatorRef.current = node;
        assignRef(ref, node);
      }}
      className={`resizable-separator${disabled ? ' is-disabled' : ''}`}
      role="separator"
      aria-label={label}
      aria-orientation="vertical"
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      aria-disabled={disabled || undefined}
      tabIndex={disabled ? -1 : 0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={(event) => {
        if (dragRef.current?.pointerId === event.pointerId) finishDrag();
      }}
      onLostPointerCapture={(event) => {
        if (dragRef.current?.pointerId === event.pointerId) finishDrag();
      }}
      onDoubleClick={() => {
        if (disabled) return;
        if (onDoubleClick) {
          onDoubleClick();
          return;
        }
        const next = clamp(defaultValue, min, max);
        dragValueRef.current = next;
        onChange(next);
        onCommit?.(next);
      }}
      onKeyDown={onKeyDown}
    >
      <span aria-hidden="true" />
    </div>
  );
}
