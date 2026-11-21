import { ChevronRight, X } from 'lucide-react';
import type { RefObject } from 'react';
import { Tab, TabList } from 'react-aria-components';
import type { UtilityView } from '../../state/store';

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
    <header className="utility-header">
      <div className="utility-header-title">
        <h2>Inspector</h2>
      </div>
      {showCollapse ? (
        <button
          ref={collapseButtonRef}
          className="icon-button icon-button-boxed utility-collapse-button"
          type="button"
          aria-label="Collapse Inspector"
          title="Collapse Inspector"
          onClick={onCollapse}
        >
          <ChevronRight size={17} aria-hidden="true" />
        </button>
      ) : (
        <button
          className="icon-button utility-close-button"
          type="button"
          aria-label="Close utility"
          title="Close utility"
          onClick={onClose}
        >
          <X size={16} aria-hidden="true" />
        </button>
      )}
      <TabList className="utility-view-tabs" aria-label="Utility views">
        <Tab id="details">
          <span>Details</span>
        </Tab>
        <Tab id="diagnostics">
          <span>Runtime</span>
        </Tab>
      </TabList>
    </header>
  );
}
