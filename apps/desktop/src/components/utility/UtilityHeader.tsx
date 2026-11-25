import { ChevronRight, X } from 'lucide-react';
import type { RefObject } from 'react';
import type { UtilityView } from '../../state/store';
import { Button } from '../ui/button';
import { TabsIndicator, TabsList, TabsTrigger } from '../ui/tabs';

interface UtilityHeaderProps {
  activeView: UtilityView;
  presentationMode: 'docked' | 'overlay';
  collapseButtonRef?: RefObject<HTMLButtonElement | null> | undefined;
  onCollapse?: () => void;
  onClose: () => void;
}

export function UtilityHeader({
  presentationMode,
  collapseButtonRef,
  onCollapse,
  onClose,
}: UtilityHeaderProps) {
  const showCollapse = presentationMode === 'docked' && onCollapse !== undefined;

  return (
    <header
      data-slot="utility-header"
      className="grid w-full min-w-0 flex-none grid-cols-[minmax(0,1fr)_32px] grid-rows-[32px_auto] items-center gap-x-2 gap-y-1 px-3.5 pt-1"
    >
      <div className="col-start-1 row-start-1 min-w-0">
        <h2 className="m-0 text-sm font-semibold leading-[1.4] text-foreground">Inspector</h2>
      </div>
      {showCollapse ? (
        <Button
          variant="ghost"
          size="icon-sm"
          ref={collapseButtonRef}
          className="col-start-2 row-start-1 size-[30px] justify-self-end p-0 hover:bg-control-hover"
          type="button"
          aria-label="Collapse Inspector"
          title="Collapse Inspector"
          onClick={onCollapse}
        >
          <ChevronRight size={17} aria-hidden="true" />
        </Button>
      ) : (
        <Button
          variant="ghost"
          size="icon-sm"
          className="col-start-2 row-start-1 size-[30px] justify-self-end p-0 hover:bg-control-hover"
          type="button"
          aria-label="Close utility"
          title="Close utility"
          onClick={onClose}
        >
          <X size={16} aria-hidden="true" />
        </Button>
      )}
      <TabsList
        className="col-span-2 row-start-2 w-full"
        aria-label="Utility views"
        variant="line"
        activateOnFocus
      >
        <TabsTrigger value="details">
          <span>Details</span>
        </TabsTrigger>
        <TabsTrigger value="diagnostics">
          <span>Runtime</span>
        </TabsTrigger>
        <TabsIndicator />
      </TabsList>
    </header>
  );
}
