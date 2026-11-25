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
  selector: string;
  matchCount: number;
  placement: 'add-songs-trigger' | 'add-songs-menu' | null;
  shape: FocusGuardShapeEvidence | null;
}

export interface FocusTraversalEvidence {
  key: 'Tab' | 'Shift+Tab';
  originId: string;
  startedAt: number;
  menuClosed: boolean;
  events: Array<{
    type: 'focusin' | 'focusout';
    id: string;
    timestamp: number;
    sentinel: boolean;
  }>;
  destination: {
    id: string;
    body: boolean;
    hidden: boolean;
    disabled: boolean;
    sentinel: boolean;
    visible: boolean;
    timestamp: number;
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

function isExactFocusGuard(
  shape: FocusGuardShapeEvidence | null,
  placement: FocusGuardSelectorEvidence['placement'],
): boolean {
  if (!shape) return false;
  return (
    shape.tagName === 'SPAN' &&
    shape.marker &&
    ((placement === 'add-songs-trigger' &&
      (shape.dataType === null || shape.dataType === 'outside')) ||
      (placement === 'add-songs-menu' && shape.dataType === 'inside')) &&
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

function hasSafeTraversalEvidence(traversals: FocusTraversalEvidence[]): boolean {
  if (traversals.length !== 2) return false;
  const forward = traversals.find((traversal) => traversal.key === 'Tab');
  const backward = traversals.find((traversal) => traversal.key === 'Shift+Tab');
  if (!forward || !backward) return false;
  if (
    !forward.menuClosed ||
    !backward.menuClosed ||
    forward.destination.id === forward.originId ||
    backward.destination.id === backward.originId ||
    forward.destination.id === backward.destination.id
  ) {
    return false;
  }
  for (const traversal of traversals) {
    const { destination, events } = traversal;
    if (
      destination.body ||
      destination.hidden ||
      destination.disabled ||
      destination.sentinel ||
      !destination.visible ||
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
    const sentinelFocus = events.filter((event) => event.type === 'focusin' && event.sentinel);
    if (
      sentinelFocus.some((entry) => {
        const exit = events.find(
          (event) => event.type === 'focusout' && event.sentinel && event.id === entry.id,
        );
        return !exit || exit.timestamp < entry.timestamp || exit.timestamp - entry.timestamp > 1000;
      })
    ) {
      return false;
    }
  }
  return true;
}

export function classifyBaseUiFocusGuardFinding(
  violation: AxeFocusViolation,
  selectorEvidence: FocusGuardSelectorEvidence[],
  traversals: FocusTraversalEvidence[],
): { adjudicated: boolean; reason: string } {
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
  for (const selector of selectors) {
    const evidence = selectorEvidence.find((entry) => entry.selector === selector);
    if (
      evidence?.matchCount !== 1 ||
      evidence.placement === null ||
      !isExactFocusGuard(evidence.shape, evidence.placement)
    ) {
      return {
        adjudicated: false,
        reason: 'Every Axe target must resolve to one exact Base UI focus guard.',
      };
    }
  }
  if (!hasSafeTraversalEvidence(traversals)) {
    return {
      adjudicated: false,
      reason: 'Tab and Shift+Tab must leave the menu through visible, enabled destinations.',
    };
  }
  return {
    adjudicated: true,
    reason:
      'Base UI 1.9.0 focus guards matched the exact sentinel shape and both traversal checks passed.',
  };
}
