import { Check, ChevronDown } from 'lucide-react';
import { useId, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { ListBox, ListBoxItem, Popover, type Selection } from 'react-aria-components';
import { findSelectedOption, type AppSelectOption } from './appSelectHelpers';

export type { AppSelectOption };

export interface AppSelectProps<T extends string = string> {
  id?: string | undefined;
  value: T;
  options: readonly AppSelectOption<T>[];
  onChange: (value: T) => void;
  ariaLabel?: string | undefined;
  ariaLabelledBy?: string | undefined;
  ariaDescribedBy?: string | undefined;
  listboxLabel?: string | undefined;
  density?: 'default' | 'compact' | undefined;
  disabled?: boolean | undefined;
  className?: string | undefined;
  popoverClassName?: string | undefined;
  popoverMinWidth?: number | undefined;
}

export function AppSelect<T extends string = string>({
  id,
  value,
  options,
  onChange,
  ariaLabel,
  ariaLabelledBy,
  ariaDescribedBy,
  listboxLabel,
  density = 'default',
  disabled = false,
  className,
  popoverClassName,
  popoverMinWidth,
}: AppSelectProps<T>) {
  const generatedId = useId();
  const listboxId = useId();
  const controlId = id ?? generatedId;
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);

  const selectedOption = useMemo(() => findSelectedOption(options, value), [options, value]);
  const selectedKeys = useMemo(() => new Set<string>([value]), [value]);
  const disabledKeys = useMemo(
    () =>
      new Set<string>(options.filter((option) => option.disabled).map((option) => option.value)),
    [options],
  );

  const closeAndRefocus = () => {
    setOpen(false);
    queueMicrotask(() => triggerRef.current?.focus());
  };

  const handleTriggerKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (disabled) return;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      setOpen(true);
    }
  };

  const handleSelectionChange = (selection: Selection) => {
    if (selection === 'all') return;
    const nextKey = selection.values().next().value;
    if (typeof nextKey !== 'string') return;
    const nextOption = findSelectedOption(options, nextKey as T);
    if (!nextOption || nextOption.disabled) return;
    if (nextOption.value !== value) {
      onChange(nextOption.value);
    }
    closeAndRefocus();
  };

  const handleAction = (key: string | number) => {
    if (String(key) === value) {
      closeAndRefocus();
    }
  };

  const handlePopoverKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      closeAndRefocus();
    } else if (event.key === 'Tab') {
      setOpen(false);
    }
  };

  const popoverStyle: CSSProperties | undefined = popoverMinWidth
    ? { minWidth: `max(var(--trigger-width), ${popoverMinWidth}px)` }
    : undefined;

  const triggerClasses = [
    'app-select-trigger',
    density === 'compact' ? 'is-compact' : '',
    open ? 'is-open' : '',
    className ?? '',
  ]
    .filter(Boolean)
    .join(' ');

  const popoverClasses = ['app-select-popover', popoverClassName ?? ''].filter(Boolean).join(' ');
  const resolvedListboxLabel = listboxLabel ?? ariaLabel ?? 'Options';

  return (
    <>
      <button
        ref={triggerRef}
        id={controlId}
        type="button"
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listboxId : undefined}
        aria-label={ariaLabel}
        aria-labelledby={ariaLabelledBy}
        aria-describedby={ariaDescribedBy}
        data-value={value}
        data-open={open ? 'true' : undefined}
        className={triggerClasses}
        disabled={disabled}
        onClick={() => {
          if (!disabled) setOpen((current) => !current);
        }}
        onKeyDown={handleTriggerKeyDown}
      >
        <span className="app-select-trigger-content">
          {selectedOption?.swatches && selectedOption.swatches.length > 0 && (
            <span className="app-select-swatches" aria-hidden="true">
              {selectedOption.swatches.map((hex) => (
                <span
                  key={`${selectedOption.value}-swatch-${hex}`}
                  className="app-select-swatch"
                  style={{ backgroundColor: hex }}
                />
              ))}
            </span>
          )}
          <span className="app-select-value">
            {selectedOption?.triggerLabel ?? selectedOption?.label ?? value}
          </span>
        </span>
        <ChevronDown size={14} className="app-select-chevron" aria-hidden="true" />
      </button>

      <Popover
        triggerRef={triggerRef}
        isOpen={open}
        onOpenChange={(nextOpen) => {
          setOpen(nextOpen);
        }}
        placement="bottom start"
        offset={6}
        containerPadding={12}
        shouldFlip
        className={popoverClasses}
        {...(popoverStyle ? { style: popoverStyle } : {})}
      >
        <div onKeyDown={handlePopoverKeyDown}>
          <ListBox
            id={listboxId}
            aria-label={resolvedListboxLabel}
            className="app-select-listbox scroll-surface"
            selectionMode="single"
            disallowEmptySelection
            selectedKeys={selectedKeys}
            disabledKeys={disabledKeys}
            autoFocus
            onSelectionChange={handleSelectionChange}
            onAction={handleAction}
          >
            {options.map((option) => (
              <ListBoxItem
                key={option.value}
                id={option.value}
                textValue={option.label}
                isDisabled={option.disabled === true}
                className="app-select-option"
                data-value={option.value}
              >
                {({ isSelected }) => (
                  <>
                    <span className="app-select-option-main">
                      {option.swatches && option.swatches.length > 0 && (
                        <span className="app-select-swatches" aria-hidden="true">
                          {option.swatches.map((hex) => (
                            <span
                              key={`${option.value}-opt-swatch-${hex}`}
                              className="app-select-swatch"
                              style={{ backgroundColor: hex }}
                            />
                          ))}
                        </span>
                      )}
                      <span className="app-select-option-label">{option.label}</span>
                    </span>
                    <span className="app-select-option-meta">
                      {option.hint && <span className="app-select-option-hint">{option.hint}</span>}
                      <span className="app-select-check-slot" aria-hidden="true">
                        {isSelected && <Check size={14} className="app-select-check" />}
                      </span>
                    </span>
                  </>
                )}
              </ListBoxItem>
            ))}
          </ListBox>
        </div>
      </Popover>
    </>
  );
}
