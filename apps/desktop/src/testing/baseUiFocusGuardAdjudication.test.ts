import { describe, expect, it } from 'vitest';
import {
  classifyBaseUiFocusGuardFinding,
  type FocusGuardSelectorEvidence,
  type FocusTraversalEvidence,
} from './baseUiFocusGuardAdjudication';

const selector = 'span[data-base-ui-focus-guard]';

const sentinelEvidence: FocusGuardSelectorEvidence = {
  selector,
  matchCount: 1,
  placement: 'add-songs-trigger',
  shape: {
    tagName: 'SPAN',
    marker: true,
    dataType: null,
    ariaHidden: 'true',
    tabIndex: 0,
    role: null,
    text: '',
    childElementCount: 0,
    interactiveDescendantCount: 0,
    width: 1,
    height: 1,
    clipPath: 'inset(50%)',
    overflow: 'hidden',
    position: 'fixed',
    hiddenAncestor: false,
  },
};

const traversals: FocusTraversalEvidence[] = [
  {
    key: 'Tab',
    originId: 'menu-item',
    startedAt: 1,
    menuClosed: true,
    events: [
      { type: 'focusout', id: 'menu-item', timestamp: 5, sentinel: false },
      { type: 'focusin', id: 'after-menu', timestamp: 8, sentinel: false },
    ],
    destination: {
      id: 'after-menu',
      body: false,
      hidden: false,
      disabled: false,
      sentinel: false,
      visible: true,
      timestamp: 12,
    },
  },
  {
    key: 'Shift+Tab',
    originId: 'menu-item',
    startedAt: 10,
    menuClosed: true,
    events: [
      { type: 'focusout', id: 'menu-item', timestamp: 11, sentinel: false },
      { type: 'focusin', id: 'before-menu', timestamp: 14, sentinel: false },
    ],
    destination: {
      id: 'before-menu',
      body: false,
      hidden: false,
      disabled: false,
      sentinel: false,
      visible: true,
      timestamp: 18,
    },
  },
];

function classify(
  overrides: {
    id?: string;
    target?: unknown;
    relatedTarget?: unknown;
    impact?: string | null;
    evidence?: FocusGuardSelectorEvidence[];
    trace?: FocusTraversalEvidence[];
  } = {},
) {
  return classifyBaseUiFocusGuardFinding(
    {
      id: overrides.id ?? 'aria-hidden-focus',
      impact: overrides.impact ?? 'serious',
      nodes: [
        {
          target: overrides.target ?? [selector],
          ...(overrides.relatedTarget
            ? { any: [{ relatedNodes: [{ target: overrides.relatedTarget }] }] }
            : {}),
        },
      ],
    },
    overrides.evidence ?? [sentinelEvidence],
    overrides.trace ?? traversals,
  );
}

describe('Base UI focus guard Axe adjudication', () => {
  it('accepts only a matching sentinel with forward and reverse real traversal evidence', () => {
    expect(classify()).toMatchObject({ adjudicated: true });
  });

  it.each([
    [
      'a hidden button',
      { ...sentinelEvidence, shape: { ...sentinelEvidence.shape!, tagName: 'BUTTON' } },
    ],
    [
      'an aria-hidden ancestor with a focusable descendant',
      {
        ...sentinelEvidence,
        shape: {
          ...sentinelEvidence.shape!,
          tagName: 'DIV',
          marker: false,
          childElementCount: 1,
          interactiveDescendantCount: 1,
        },
      },
    ],
    [
      'a sentinel below a hidden ancestor',
      { ...sentinelEvidence, shape: { ...sentinelEvidence.shape!, hiddenAncestor: true } },
    ],
    [
      'a spoof marker with visible geometry',
      { ...sentinelEvidence, shape: { ...sentinelEvidence.shape!, clipPath: 'none' } },
    ],
    ['a selector resolving to duplicate DOM targets', { ...sentinelEvidence, matchCount: 2 }],
  ])('blocks %s', (_label, evidence) => {
    expect(classify({ evidence: [evidence] })).toMatchObject({ adjudicated: false });
  });

  it('blocks a mixed sentinel and interactive-target finding', () => {
    expect(classify({ target: [selector, 'button[aria-label="hidden action"]'] })).toMatchObject({
      adjudicated: false,
    });
  });

  it('blocks an otherwise matching sentinel outside the reviewed Add songs flow', () => {
    expect(classify({ evidence: [{ ...sentinelEvidence, placement: null }] })).toMatchObject({
      adjudicated: false,
    });
  });

  it('blocks a related node that does not match the sentinel', () => {
    expect(classify({ relatedTarget: ['button[aria-label="hidden action"]'] })).toMatchObject({
      adjudicated: false,
    });
  });

  it('accepts repeated Axe references only when they resolve to the same exact sentinel', () => {
    expect(classify({ relatedTarget: [selector] })).toMatchObject({ adjudicated: true });
  });

  it('blocks unrelated Axe rules and critical aria-hidden-focus findings', () => {
    expect(classify({ id: 'color-contrast' })).toMatchObject({ adjudicated: false });
    expect(classify({ impact: 'critical' })).toMatchObject({ adjudicated: false });
  });

  it('blocks stuck sentinels, body focus, hidden destinations, and looped traversal', () => {
    const stuck = traversals.map((traversal) => ({
      ...traversal,
      events: [
        ...traversal.events,
        { type: 'focusin' as const, id: 'guard', timestamp: 20, sentinel: true },
      ],
    }));
    expect(classify({ trace: stuck })).toMatchObject({ adjudicated: false });
    expect(
      classify({
        trace: traversals.map((traversal, index) =>
          index === 0
            ? { ...traversal, destination: { ...traversal.destination, body: true } }
            : traversal,
        ),
      }),
    ).toMatchObject({ adjudicated: false });
    expect(
      classify({
        trace: traversals.map((traversal, index) =>
          index === 0
            ? { ...traversal, destination: { ...traversal.destination, hidden: true } }
            : traversal,
        ),
      }),
    ).toMatchObject({ adjudicated: false });
    expect(
      classify({
        trace: traversals.map((traversal) => ({
          ...traversal,
          destination: { ...traversal.destination, id: 'same-destination' },
        })),
      }),
    ).toMatchObject({ adjudicated: false });
  });
});
