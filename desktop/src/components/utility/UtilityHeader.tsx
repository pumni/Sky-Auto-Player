import { X } from 'lucide-react';
import type { UtilityView } from '../../state/store';

interface UtilityHeaderProps {
  activeView: UtilityView;
  onClose: () => void;
  onViewChange: (view: UtilityView) => void;
}

export function UtilityHeader({ activeView, onClose, onViewChange }: UtilityHeaderProps) {
  const title = activeView === 'diagnostics' ? 'Diagnostics' : 'Song Details';
  return (
    <header className="utility-header">
      <div>
        <p className="eyebrow">UTILITY</p>
        <h2>{title}</h2>
      </div>
      <button
        className="icon-button"
        type="button"
        aria-label="Close utility"
        title="Close utility"
        onClick={onClose}
      >
        <X size={16} aria-hidden="true" />
      </button>
      <div className="utility-view-tabs" role="tablist" aria-label="Utility views">
        <button
          type="button"
          role="tab"
          aria-selected={activeView === 'details'}
          onClick={() => onViewChange('details')}
        >
          Details
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={activeView === 'diagnostics'}
          onClick={() => onViewChange('diagnostics')}
        >
          Runtime
        </button>
      </div>
    </header>
  );
}
