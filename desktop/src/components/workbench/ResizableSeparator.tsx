import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';

interface ResizableSeparatorProps {
  label: string;
  value: number;
  min: number;
  max: number;
  defaultValue: number;
  /** Direction of the pane whose width this separator owns. */
  direction?: 1 | -1;
  onChange: (value: number) => void;
  onCommit?: (value: number) => void;
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
  onChange,
  onCommit,
}: ResizableSeparatorProps) {
  const separatorRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{ pointerId: number; startX: number; startValue: number } | null>(null);
  const dragValueRef = useRef(value);
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    if (!dragging) return;
    document.body.classList.add('is-resizing');
    return () => document.body.classList.remove('is-resizing');
  }, [dragging]);

  const finishDrag = () => {
    if (!dragRef.current) return;
    dragRef.current = null;
    setDragging(false);
    onCommit?.(dragValueRef.current);
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    dragRef.current = { pointerId: event.pointerId, startX: event.clientX, startValue: value };
    dragValueRef.current = value;
    setDragging(true);
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const next = clamp(drag.startValue + direction * (event.clientX - drag.startX), min, max);
    dragValueRef.current = next;
    onChange(next);
  };

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (dragRef.current?.pointerId !== event.pointerId) return;
    finishDrag();
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
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
      className={`resizable-separator${dragging ? ' is-dragging' : ''}`}
      role="separator"
      aria-label={label}
      aria-orientation="vertical"
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={finishDrag}
      onLostPointerCapture={finishDrag}
      onDoubleClick={() => {
        dragValueRef.current = defaultValue;
        onChange(defaultValue);
        onCommit?.(defaultValue);
      }}
      onKeyDown={onKeyDown}
    >
      <span aria-hidden="true" />
    </div>
  );
}
