import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  classifyBaseUiFocusGuardFinding,
  REVIEWED_ADD_SONGS_GUARDS,
  REVIEWED_AXE_VERSION,
  REVIEWED_BASE_UI_VERSION,
  type AxeFocusViolation,
  type FocusGuardSelectorEvidence,
  type FocusTraversalEvidence,
} from './baseUiFocusGuardAdjudication';

const desktopManifest = JSON.parse(
  readFileSync(resolve(process.cwd(), 'package.json'), 'utf8'),
) as { dependencies: Record<string, string> };
const installedBaseUi = JSON.parse(
  readFileSync(resolve(process.cwd(), 'node_modules/@base-ui/react/package.json'), 'utf8'),
) as { version: string };
const installedAxe = JSON.parse(
  readFileSync(resolve(process.cwd(), 'node_modules/@axe-core/playwright/package.json'), 'utf8'),
) as { version: string };
const versions = { baseUi: installedBaseUi.version, axe: installedAxe.version };

function guardEvidence(
  snapshotId: string,
  selector: (typeof REVIEWED_ADD_SONGS_GUARDS)[number]['selector'],
  traceIdentity: string | null,
): FocusGuardSelectorEvidence {
  const expected = REVIEWED_ADD_SONGS_GUARDS.find((guard) => guard.selector === selector);
  if (!expected) throw new Error(`Unreviewed guard selector: ${selector}`);
  return {
    snapshotId,
    selector,
    matchCount: 1,
    placement: expected.placement,
    traceIdentity,
    shape: {
      tagName: 'SPAN',
      marker: true,
      dataType: expected.dataType,
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
}

function snapshotEvidence(
  snapshotId: string,
  identities: Array<string | null>,
): FocusGuardSelectorEvidence[] {
  return REVIEWED_ADD_SONGS_GUARDS.map((guard, index) =>
    guardEvidence(snapshotId, guard.selector, identities[index] ?? null),
  );
}

const forwardEvidence = snapshotEvidence('forward-menu', [
  'forward-trigger-guard',
  null,
  null,
  null,
  'forward-menu-guard',
  null,
]);
const backwardEvidence = snapshotEvidence('backward-menu', [
  'backward-guard-0',
  'backward-guard-1',
  'backward-guard-2',
  'backward-guard-3',
  'backward-guard-4',
  'backward-guard-5',
]);
const axeEvidence = snapshotEvidence('axe-menu', [
  'axe-guard-0',
  'axe-guard-1',
  'axe-guard-2',
  'axe-guard-3',
  'axe-guard-4',
  'axe-guard-5',
]);
const selectorEvidence = [...forwardEvidence, ...backwardEvidence, ...axeEvidence];

const traversals: FocusTraversalEvidence[] = [
  {
    key: 'Tab',
    snapshotId: 'forward-menu',
    originId: 'menu-item',
    startedAt: 1,
    menuClosed: true,
    events: [
      { type: 'focusout', id: 'menu-item', timestamp: 5, sentinel: false, guardPlacement: null },
      {
        type: 'focusin',
        id: 'forward-menu-guard',
        timestamp: 6,
        sentinel: true,
        guardPlacement: 'add-songs-menu',
      },
      {
        type: 'focusout',
        id: 'forward-menu-guard',
        timestamp: 7,
        sentinel: true,
        guardPlacement: 'add-songs-menu',
      },
      {
        type: 'focusin',
        id: 'forward-trigger-guard',
        timestamp: 8,
        sentinel: true,
        guardPlacement: 'add-songs-trigger',
      },
      {
        type: 'focusout',
        id: 'forward-trigger-guard',
        timestamp: 9,
        sentinel: true,
        guardPlacement: 'add-songs-trigger',
      },
      {
        type: 'focusin',
        id: 'empty-browse-button',
        timestamp: 10,
        sentinel: false,
        guardPlacement: null,
      },
    ],
    destination: {
      id: 'empty-browse-button',
      body: false,
      hidden: false,
      disabled: false,
      sentinel: false,
      visible: true,
      timestamp: 12,
      logicalTarget: 'empty-playlist-browse',
      role: 'button',
      dataSlot: 'button',
      accessibleName: 'Browse Built-in Songs…',
    },
  },
  {
    key: 'Shift+Tab',
    snapshotId: 'backward-menu',
    originId: 'menu-item-reverse',
    startedAt: 20,
    menuClosed: true,
    events: [
      {
        type: 'focusout',
        id: 'menu-item-reverse',
        timestamp: 21,
        sentinel: false,
        guardPlacement: null,
      },
      {
        type: 'focusin',
        id: 'playlist-add-trigger',
        timestamp: 22,
        sentinel: false,
        guardPlacement: null,
      },
    ],
    destination: {
      id: 'playlist-add-trigger',
      body: false,
      hidden: false,
      disabled: false,
      sentinel: false,
      visible: true,
      timestamp: 24,
      logicalTarget: 'playlist-add-trigger',
      role: 'button',
      dataSlot: 'playlist-add-songs-button',
      accessibleName: 'Add songs',
    },
  },
];

function violation(overrides: Partial<AxeFocusViolation> = {}): AxeFocusViolation {
  const nodes = REVIEWED_ADD_SONGS_GUARDS.map(({ selector }) => ({ target: [selector] }));
  return {
    id: 'aria-hidden-focus',
    impact: 'serious',
    nodes,
    ...overrides,
  };
}

function classify(
  overrides: {
    finding?: Partial<AxeFocusViolation>;
    evidence?: FocusGuardSelectorEvidence[];
    trace?: FocusTraversalEvidence[];
    toolVersions?: { baseUi: string; axe: string };
  } = {},
) {
  return classifyBaseUiFocusGuardFinding(
    violation(overrides.finding),
    overrides.evidence ?? selectorEvidence,
    overrides.trace ?? traversals,
    overrides.toolVersions ?? versions,
  );
}

describe('Base UI focus guard Axe adjudication', () => {
  it('pins the installed packages and direct dependency to the reviewed versions', () => {
    expect(desktopManifest.dependencies['@base-ui/react']).toBe(REVIEWED_BASE_UI_VERSION);
    expect(installedBaseUi.version).toBe(REVIEWED_BASE_UI_VERSION);
    expect(installedAxe.version).toBe(REVIEWED_AXE_VERSION);
  });

  it('accepts the six reviewed guards with correlated real traversal and destinations', () => {
    expect(classify()).toMatchObject({ adjudicated: true });
  });

  it('fails closed when either reviewed package version drifts', () => {
    expect(
      classify({ toolVersions: { baseUi: '1.9.1', axe: REVIEWED_AXE_VERSION } }),
    ).toMatchObject({ adjudicated: false });
    expect(
      classify({ toolVersions: { baseUi: REVIEWED_BASE_UI_VERSION, axe: '4.15.0' } }),
    ).toMatchObject({ adjudicated: false });
  });

  it('blocks missing, extra, mixed, or remapped selectors', () => {
    expect(classify({ evidence: selectorEvidence.slice(0, -1) })).toMatchObject({
      adjudicated: false,
    });
    expect(
      classify({
        evidence: [
          ...selectorEvidence,
          { ...selectorEvidence[0]!, selector: 'button[aria-label="unexpected"]' },
        ],
      }),
    ).toMatchObject({ adjudicated: false });
    expect(
      classify({
        finding: {
          nodes: [...violation().nodes, { target: ['button[aria-label="hidden action"]'] }],
        },
      }),
    ).toMatchObject({ adjudicated: false });
    expect(
      classify({
        finding: {
          nodes: [...violation().nodes, { target: [REVIEWED_ADD_SONGS_GUARDS[0]!.selector] }],
        },
      }),
    ).toMatchObject({ adjudicated: false });
  });

  it.each([
    [
      'a hidden button',
      (entry: FocusGuardSelectorEvidence) => ({
        ...entry,
        shape: { ...entry.shape!, tagName: 'BUTTON' },
      }),
    ],
    [
      'a visible spoof marker',
      (entry: FocusGuardSelectorEvidence) => ({
        ...entry,
        shape: { ...entry.shape!, clipPath: 'none' },
      }),
    ],
    [
      'a hidden ancestor',
      (entry: FocusGuardSelectorEvidence) => ({
        ...entry,
        shape: { ...entry.shape!, hiddenAncestor: true },
      }),
    ],
    [
      'an unexpected placement',
      (entry: FocusGuardSelectorEvidence) => ({ ...entry, placement: 'add-songs-menu' as const }),
    ],
  ])('blocks %s', (_label, alter) => {
    const changed = selectorEvidence.map((entry, index) => (index === 0 ? alter(entry) : entry));
    expect(classify({ evidence: changed })).toMatchObject({ adjudicated: false });
  });

  it('blocks duplicate DOM targets and mixed Axe nodes', () => {
    const changed = selectorEvidence.map((entry, index) =>
      index === 0 ? { ...entry, matchCount: 2 } : entry,
    );
    expect(classify({ evidence: changed })).toMatchObject({ adjudicated: false });
    expect(
      classify({
        finding: {
          nodes: [...violation().nodes, { target: ['button[aria-label="hidden action"]'] }],
        },
      }),
    ).toMatchObject({ adjudicated: false });
  });

  it('blocks uncorrelated guard identities and wrong placements during keyboard traversal', () => {
    const wrongPlacement = traversals.map((traversal, index) =>
      index === 0
        ? {
            ...traversal,
            events: traversal.events.map((event) =>
              event.id === 'forward-menu-guard'
                ? { ...event, guardPlacement: 'add-songs-trigger' as const }
                : event,
            ),
          }
        : traversal,
    );
    expect(classify({ trace: wrongPlacement })).toMatchObject({ adjudicated: false });
    const unknownIdentity = traversals.map((traversal, index) =>
      index === 0
        ? {
            ...traversal,
            events: traversal.events.map((event) =>
              event.id === 'forward-menu-guard' ? { ...event, id: 'unreviewed-guard' } : event,
            ),
          }
        : traversal,
    );
    expect(classify({ trace: unknownIdentity })).toMatchObject({ adjudicated: false });
  });

  it('blocks wrong, missing, hidden, or stuck focus destinations', () => {
    const wrongDestination = traversals.map((traversal, index) =>
      index === 0
        ? {
            ...traversal,
            destination: { ...traversal.destination, accessibleName: 'Retry' },
          }
        : traversal,
    );
    expect(classify({ trace: wrongDestination })).toMatchObject({ adjudicated: false });
    expect(
      classify({
        trace: traversals.map((traversal, index) =>
          index === 0 ? { ...traversal, menuClosed: false } : traversal,
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
        trace: traversals.map((traversal, index) =>
          index === 0
            ? {
                ...traversal,
                events: traversal.events.map((event) =>
                  event.id === 'forward-menu-guard' && event.type === 'focusout'
                    ? { ...event, timestamp: 1008 }
                    : event,
                ),
              }
            : traversal,
        ),
      }),
    ).toMatchObject({ adjudicated: false });
  });

  it('blocks unrelated Axe rules, critical findings, and empty targets', () => {
    expect(classify({ finding: { id: 'color-contrast' } })).toMatchObject({ adjudicated: false });
    expect(classify({ finding: { impact: 'critical' } })).toMatchObject({ adjudicated: false });
    expect(classify({ finding: { nodes: [] } })).toMatchObject({ adjudicated: false });
  });
});
