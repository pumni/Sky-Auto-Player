import { Heart } from 'lucide-react';

export function TrackTableHeader() {
  return (
    <div className="track-table-row track-table-header" role="row" aria-rowindex={1}>
      <span className="track-cell track-cell-index" role="columnheader">
        #
      </span>
      <span className="track-cell track-cell-title" role="columnheader">
        Title
      </span>
      <span className="track-cell track-cell-liked" role="columnheader" aria-label="Liked">
        <Heart size={14} aria-hidden="true" />
      </span>
      <span className="track-cell track-cell-notes" role="columnheader">
        Notes
      </span>
      <span className="track-cell track-cell-duration" role="columnheader">
        Duration
      </span>
    </div>
  );
}
