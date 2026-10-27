import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';

interface ResizableSeparatorProps {
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
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}

export function ResizableSeparator({
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
}: ResizableSeparatorProps) {
  const separatorRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{
    pointerId: number;
    startX: number;
    startValue: number;
    lastX: number;
  } | null>(null);
  const dragValueRef = useRef(value);
  const frameRef = useRef<number | null>(null);
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    if (!dragging) return;
    document.body.classList.add('is-resizing');
    return () => document.body.classList.remove('is-resizing');
  }, [dragging]);

  useEffect(
    () => () => {
      if (frameRef.current !== null) window.cancelAnimationFrame(frameRef.current);
      dragRef.current = null;
      document.body.classList.remove('is-resizing');
    },
    [],
  );

  const applyLatest = () => {
    const drag = dragRef.current;
    if (!drag) return;
    const next = clamp(drag.startValue + direction * (drag.lastX - drag.startX), min, max);
    if (next === dragValueRef.current) return;
    dragValueRef.current = next;
    onChange(next);
  };

  // A pending frame must use current bounds/callbacks after a viewport change.
  useEffect(() => {
    if (frameRef.current !== null) window.cancelAnimationFrame(frameRef.current);
    frameRef.current = null;
    if (!dragRef.current) return;
    frameRef.current = window.requestAnimationFrame(() => {
      frameRef.current = null;
      applyLatest();
    });
  });

  const finishDrag = () => {
    if (!dragRef.current) return;
    if (frameRef.current !== null) window.cancelAnimationFrame(frameRef.current);
    frameRef.current = null;
    applyLatest();
    dragRef.current = null;
    setDragging(false);
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
    setDragging(true);
    onDragStart?.();
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    drag.lastX = event.clientX;
    if (frameRef.current !== null) return;
    frameRef.current = window.requestAnimationFrame(() => {
      frameRef.current = null;
      applyLatest();
    });
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
      ref={separatorRef}
      className={`resizable-separator${dragging ? ' is-dragging' : ''}${disabled ? ' is-disabled' : ''}`}
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
