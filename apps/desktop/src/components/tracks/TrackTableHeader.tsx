import { Heart } from 'lucide-react';

export function TrackTableHeader() {
  return (
    <div
      className="sticky top-0 z-[1] grid h-8 min-w-0 grid-cols-[36px_minmax(0,1fr)_32px_72px_76px] items-center gap-2 bg-surface px-[18px] text-[11px] font-semibold uppercase text-text-tertiary after:absolute after:bottom-0 after:left-[18px] after:right-[18px] after:h-px after:bg-border after:content-[''] @max-[520px]/track-browser:grid-cols-[30px_minmax(0,1fr)_32px_76px] @max-[520px]/track-browser:px-3 @max-[520px]/track-browser:after:left-3 @max-[520px]/track-browser:after:right-3 @max-[440px]/track-browser:grid-cols-[26px_minmax(0,1fr)_32px_70px] @max-[440px]/track-browser:gap-[5px] @max-[440px]/track-browser:px-2 @max-[440px]/track-browser:after:left-2 @max-[440px]/track-browser:after:right-2"
      role="row"
      aria-rowindex={1}
    >
      <span className="min-w-0 text-center tabular-nums" role="columnheader">
        #
      </span>
      <span className="min-w-0" role="columnheader">
        Title
      </span>
      <span
        className="flex min-w-0 items-center justify-center"
        role="columnheader"
        aria-label="Liked"
      >
        <Heart size={14} aria-hidden="true" />
      </span>
      <span
        className="min-w-0 text-right tabular-nums @max-[520px]/track-browser:hidden"
        role="columnheader"
      >
        Notes
      </span>
      <span className="min-w-0 text-right tabular-nums" role="columnheader">
        Duration
      </span>
    </div>
  );
}
