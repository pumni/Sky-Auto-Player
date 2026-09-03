import { X } from 'lucide-react';
import { Tab, TabList } from 'react-aria-components';
import type { UtilityView } from '../../state/store';

interface UtilityHeaderProps {
  activeView: UtilityView;
  onClose: () => void;
}

export function UtilityHeader({ activeView, onClose }: UtilityHeaderProps) {
  const title = activeView === 'diagnostics' ? 'Diagnostics' : 'Song Details';
  return (
    <header className="utility-header">
      <div className="utility-header-title">
        <h2>{title}</h2>
      </div>
      <button
        className="icon-button utility-close-button"
        type="button"
        aria-label="Close utility"
        title="Close utility"
        onClick={onClose}
      >
        <X size={16} aria-hidden="true" />
      </button>
      <TabList className="utility-view-tabs" aria-label="Utility views">
        <Tab id="details">Details</Tab>
        <Tab id="diagnostics">Runtime</Tab>
      </TabList>
    </header>
  );
}
