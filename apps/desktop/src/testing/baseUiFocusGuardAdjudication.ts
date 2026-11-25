export const REVIEWED_BASE_UI_VERSION = '1.9.0';
export const REVIEWED_AXE_VERSION = '4.14.0';

export type FocusGuardPlacement = 'add-songs-trigger' | 'add-songs-menu';

export interface FocusGuardShapeEvidence {
  tagName: string;
  marker: boolean;
  dataType: string | null;
  ariaHidden: string | null;
  tabIndex: number;
  role: string | null;
  text: string;
  childElementCount: number;
  interactiveDescendantCount: number;
  width: number;
  height: number;
  clipPath: string;
  overflow: string;
  position: string;
  hiddenAncestor: boolean;
}

export interface FocusGuardSelectorEvidence {
  snapshotId: string;
  selector: string;
  matchCount: number;
  placement: FocusGuardPlacement | null;
  traceIdentity: string | null;
  shape: FocusGuardShapeEvidence | null;
}

export interface FocusTraversalEvidence {
  snapshotId: string;
  key: 'Tab' | 'Shift+Tab';
  originId: string;
  startedAt: number;
  menuClosed: boolean;
  events: Array<{
    type: 'focusin' | 'focusout';
    id: string;
    timestamp: number;
    sentinel: boolean;
    guardPlacement: FocusGuardPlacement | null;
  }>;
  destination: {
    id: string;
    body: boolean;
    hidden: boolean;
    disabled: boolean;
    sentinel: boolean;
    visible: boolean;
    timestamp: number;
    logicalTarget:
      | 'empty-playlist-browse'
      | 'playlist-add-trigger'
      | 'track-grid'
      | 'library-navigation'
      | 'other';
    role: string | null;
    dataSlot: string | null;
    accessibleName: string;
  };
}

interface AxeRelatedNode {
  target?: unknown;
}

interface AxeNode {
  target?: unknown;
  all?: Array<{ relatedNodes?: AxeRelatedNode[] }>;
  any?: Array<{ relatedNodes?: AxeRelatedNode[] }>;
  none?: Array<{ relatedNodes?: AxeRelatedNode[] }>;
}

export interface AxeFocusViolation {
  id: string;
  impact: string | null;
  nodes: AxeNode[];
}

interface ReviewedGuard {
  selector: string;
  placement: FocusGuardPlacement;
  dataType: string | null;
}

export const REVIEWED_ADD_SONGS_GUARDS: readonly ReviewedGuard[] = [
  {
    selector: '.flex-none.flex.items-center > span[data-base-ui-focus-guard=""]:nth-child(1)',
    placement: 'add-songs-trigger',
    dataType: null,
  },
  {
    selector: '.flex-none.flex.items-center > span[data-base-ui-focus-guard=""]:nth-child(3)',
    placement: 'add-songs-trigger',
    dataType: null,
  },
  {
    selector: 'span[data-type="outside"][data-base-ui-focus-guard=""]:nth-child(4)',
    placement: 'add-songs-trigger',
    dataType: 'outside',
  },
  {
    selector: 'span[data-type="outside"][data-base-ui-focus-guard=""]:nth-child(6)',
    placement: 'add-songs-trigger',
    dataType: 'outside',
  },
  {
    selector: 'span[data-type="inside"][data-base-ui-focus-guard=""]:nth-child(1)',
    placement: 'add-songs-menu',
    dataType: 'inside',
  },
  {
    selector: 'span[data-type="inside"][data-base-ui-focus-guard=""]:nth-child(3)',
    placement: 'add-songs-menu',
    dataType: 'inside',
  },
];

function targetSelectors(target: unknown): string[] | null {
  if (!Array.isArray(target) || target.length === 0) return null;
  if (
    !target.every(
      (selector): selector is string => typeof selector === 'string' && selector.length > 0,
    )
  ) {
    return null;
  }
  return target;
}

function isExactFocusGuard(evidence: FocusGuardSelectorEvidence, expected: ReviewedGuard): boolean {
  const shape = evidence.shape;
  if (!shape) return false;
  return (
    evidence.placement === expected.placement &&
    shape.tagName === 'SPAN' &&
    shape.marker &&
    shape.dataType === expected.dataType &&
    shape.ariaHidden === 'true' &&
    shape.tabIndex === 0 &&
    shape.role === null &&
    shape.text.length === 0 &&
    shape.childElementCount === 0 &&
    shape.interactiveDescendantCount === 0 &&
    Math.abs(shape.width - 1) <= 0.1 &&
    Math.abs(shape.height - 1) <= 0.1 &&
    shape.clipPath.includes('inset(50%)') &&
    shape.overflow === 'hidden' &&
    shape.position === 'fixed' &&
    !shape.hiddenAncestor
  );
}

function hasSafeTraversalEvidence(
  traversals: FocusTraversalEvidence[],
  selectorEvidence: FocusGuardSelectorEvidence[],
): boolean {
  if (traversals.length !== 2) return false;
  const forward = traversals.find((traversal) => traversal.key === 'Tab');
  const backward = traversals.find((traversal) => traversal.key === 'Shift+Tab');
  if (!forward || !backward) return false;
  if (
    !forward.menuClosed ||
    !backward.menuClosed ||
    forward.destination.logicalTarget !== 'empty-playlist-browse' ||
    forward.destination.role !== 'button' ||
    forward.destination.accessibleName !== 'Browse Built-in Songs…' ||
    backward.destination.logicalTarget !== 'playlist-add-trigger' ||
    backward.destination.role !== 'button' ||
    backward.destination.accessibleName !== 'Add songs' ||
    forward.destination.id === forward.originId ||
    backward.destination.id === backward.originId ||
    forward.destination.id === backward.destination.id
  ) {
    return false;
  }

  for (const traversal of traversals) {
    const traversalEvidence = selectorEvidence.filter(
      (evidence) => evidence.snapshotId === traversal.snapshotId,
    );
    if (traversalEvidence.length !== REVIEWED_ADD_SONGS_GUARDS.length) return false;
    const guardPlacements = new Map<string, FocusGuardPlacement>();
    for (const evidence of traversalEvidence) {
      if (evidence.traceIdentity === null) continue;
      if (guardPlacements.has(evidence.traceIdentity) || evidence.placement === null) return false;
      guardPlacements.set(evidence.traceIdentity, evidence.placement);
    }

    const { destination, events } = traversal;
    if (
      destination.body ||
      destination.hidden ||
      destination.disabled ||
      destination.sentinel ||
      !destination.visible ||
      destination.accessibleName.trim().length === 0 ||
      destination.timestamp < traversal.startedAt ||
      destination.timestamp - traversal.startedAt > 1000 ||
      !events.some((event) => event.type === 'focusin') ||
      !events.some((event) => event.type === 'focusout')
    ) {
      return false;
    }
    const reachedDestination = events.find(
      (event) => event.type === 'focusin' && event.id === destination.id,
    );
    if (
      !reachedDestination ||
      reachedDestination.timestamp < traversal.startedAt ||
      reachedDestination.timestamp - traversal.startedAt > 1000
    ) {
      return false;
    }
    for (const event of events.filter((entry) => entry.sentinel)) {
      if (event.guardPlacement !== guardPlacements.get(event.id)) return false;
      if (
        event.type === 'focusin' &&
        !events.some(
          (exit) =>
            exit.type === 'focusout' &&
            exit.sentinel &&
            exit.id === event.id &&
            exit.timestamp >= event.timestamp &&
            exit.timestamp - event.timestamp <= 1000,
        )
      ) {
        return false;
      }
    }
  }
  return true;
}

export function classifyBaseUiFocusGuardFinding(
  violation: AxeFocusViolation,
  selectorEvidence: FocusGuardSelectorEvidence[],
  traversals: FocusTraversalEvidence[],
  versions: { baseUi: string; axe: string },
): { adjudicated: boolean; reason: string } {
  if (versions.baseUi !== REVIEWED_BASE_UI_VERSION || versions.axe !== REVIEWED_AXE_VERSION) {
    return {
      adjudicated: false,
      reason: `Review is pinned to Base UI ${REVIEWED_BASE_UI_VERSION} and Axe ${REVIEWED_AXE_VERSION}.`,
    };
  }
  if (violation.id !== 'aria-hidden-focus' || violation.impact !== 'serious') {
    return {
      adjudicated: false,
      reason: 'Only a serious aria-hidden-focus finding can be reviewed.',
    };
  }
  if (violation.nodes.length === 0) {
    return { adjudicated: false, reason: 'The Axe finding has no result nodes.' };
  }

  const selectors: string[] = [];
  for (const node of violation.nodes) {
    const targets = targetSelectors(node.target);
    if (targets?.length !== 1) {
      return { adjudicated: false, reason: 'Each Axe result node must identify one exact target.' };
    }
    selectors.push(...targets);
    for (const checks of [node.all, node.any, node.none]) {
      for (const check of checks ?? []) {
        for (const related of check.relatedNodes ?? []) {
          const relatedTargets = targetSelectors(related.target);
          if (relatedTargets?.length !== 1) {
            return {
              adjudicated: false,
              reason: 'Every related Axe node must identify one exact target.',
            };
          }
          selectors.push(...relatedTargets);
        }
      }
    }
  }

  const uniqueSelectors = new Set(selectors);
  const reviewedSelectors = new Set(REVIEWED_ADD_SONGS_GUARDS.map(({ selector }) => selector));
  if (
    uniqueSelectors.size !== reviewedSelectors.size ||
    [...reviewedSelectors].some((selector) => !uniqueSelectors.has(selector))
  ) {
    return {
      adjudicated: false,
      reason: 'The full finding must match the six reviewed Add songs focus-guard selectors.',
    };
  }
  // Each real keyboard traversal and the open-menu Axe scan gets its own DOM snapshot.
  if (selectorEvidence.length !== REVIEWED_ADD_SONGS_GUARDS.length * (traversals.length + 1)) {
    return {
      adjudicated: false,
      reason: 'Every keyboard traversal and the Axe scan must record six reviewed focus guards.',
    };
  }

  const snapshotIds = new Set(selectorEvidence.map(({ snapshotId }) => snapshotId));
  if (snapshotIds.size !== traversals.length + 1) {
    return {
      adjudicated: false,
      reason: 'Each traversal and the Axe scan must identify a separate open-menu DOM snapshot.',
    };
  }
  for (const snapshotId of snapshotIds) {
    const snapshotEvidence = selectorEvidence.filter((entry) => entry.snapshotId === snapshotId);
    if (snapshotEvidence.length !== REVIEWED_ADD_SONGS_GUARDS.length) {
      return {
        adjudicated: false,
        reason: `Snapshot ${snapshotId} does not contain exactly the six reviewed Add songs focus guards.`,
      };
    }
    for (const expected of REVIEWED_ADD_SONGS_GUARDS) {
      const matches = snapshotEvidence.filter((entry) => entry.selector === expected.selector);
      const evidence = matches[0];
      if (
        matches.length !== 1 ||
        evidence?.matchCount !== 1 ||
        !isExactFocusGuard(evidence, expected)
      ) {
        return {
          adjudicated: false,
          reason: `The reviewed ${expected.placement} guard ${expected.selector} changed shape or placement.`,
        };
      }
    }
  }

  if (!hasSafeTraversalEvidence(traversals, selectorEvidence)) {
    return {
      adjudicated: false,
      reason:
        'Real Tab must reach Browse Built-in Songs and Shift+Tab must return to Add songs, clearing each reached guard.',
    };
  }
  return {
    adjudicated: true,
    reason:
      'Base UI 1.9.0 and Axe 4.14.0 matched the six reviewed guard locations and the measured Tab/Shift+Tab destinations.',
  };
}
