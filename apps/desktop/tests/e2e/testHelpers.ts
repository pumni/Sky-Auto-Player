import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import AxeBuilder from '@axe-core/playwright';
import { expect, type Locator, type Page, type TestInfo } from '@playwright/test';
import {
  classifyBaseUiFocusGuardFinding,
  REVIEWED_ADD_SONGS_GUARDS,
  REVIEWED_AXE_VERSION,
  REVIEWED_BASE_UI_VERSION,
  type FocusGuardPlacement,
  type FocusGuardSelectorEvidence,
  type FocusTraversalEvidence,
} from '../../src/testing/baseUiFocusGuardAdjudication';

function installedPackageVersion(...segments: string[]): string {
  const packageJson = JSON.parse(
    readFileSync(resolve(process.cwd(), 'node_modules', ...segments, 'package.json'), 'utf8'),
  ) as { version?: unknown };
  if (typeof packageJson.version !== 'string') {
    throw new Error(`Package ${segments.join('/')} has no installed version.`);
  }
  return packageJson.version;
}

const reviewedAxeVersions = {
  baseUi: installedPackageVersion('@base-ui', 'react'),
  axe: installedPackageVersion('@axe-core', 'playwright'),
};

export const palettes = [
  'graphite',
  'aurora',
  'minimalist',
  'slate',
  'cyberpunk',
  'classic',
] as const;
export type PaletteId = (typeof palettes)[number];

// Independent E2E oracle for the approved Inspector breakpoint. Keep this
// separate from the production layout helper so browser tests can detect drift.
export function expectedUtilityPresentation(viewportWidth: number): 'docked' | 'overlay' {
  return viewportWidth >= 764 ? 'docked' : 'overlay';
}

export async function expectNoSeriousAccessibilityViolations(page: Page) {
  const results = await new AxeBuilder({ page }).analyze();
  expect(
    results.violations.filter(
      (violation) => violation.impact === 'critical' || violation.impact === 'serious',
    ),
  ).toEqual([]);
}

interface AxeRelatedNode {
  target?: unknown;
}

interface AxeResultNode {
  target?: unknown;
  all?: Array<{ relatedNodes?: AxeRelatedNode[] }>;
  any?: Array<{ relatedNodes?: AxeRelatedNode[] }>;
  none?: Array<{ relatedNodes?: AxeRelatedNode[] }>;
}

interface AxeViolation {
  id: string;
  impact: string | null;
  nodes: AxeResultNode[];
}

function axeSelectors(target: unknown): string[] {
  if (!Array.isArray(target)) return [];
  return target.filter((selector): selector is string => typeof selector === 'string');
}

function selectorsForViolation(violation: AxeViolation): string[] {
  const selectors: string[] = [];
  for (const node of violation.nodes) {
    selectors.push(...axeSelectors(node.target));
    for (const checks of [node.all, node.any, node.none]) {
      for (const check of checks ?? []) {
        for (const related of check.relatedNodes ?? []) {
          selectors.push(...axeSelectors(related.target));
        }
      }
    }
  }
  return [...new Set(selectors)];
}

async function readFocusGuardSelectorEvidence(
  page: Page,
  selector: string,
  snapshotId: string,
): Promise<FocusGuardSelectorEvidence> {
  const locator = page.locator(selector);
  const matchCount = await locator.count().catch(() => 0);
  if (matchCount !== 1) {
    return { snapshotId, selector, matchCount, placement: null, traceIdentity: null, shape: null };
  }
  const measured = await locator.evaluate((node) => {
    const element = node as HTMLElement;
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    const hiddenAncestor = (() => {
      for (let ancestor = element.parentElement; ancestor; ancestor = ancestor.parentElement) {
        const ancestorStyle = getComputedStyle(ancestor);
        if (
          ancestor.hidden ||
          ancestor.inert ||
          ancestor.getAttribute('aria-hidden') === 'true' ||
          ancestorStyle.display === 'none' ||
          ancestorStyle.visibility === 'hidden'
        ) {
          return true;
        }
      }
      return false;
    })();
    const interactiveDescendants = element.querySelectorAll(
      'a[href],button,input,select,textarea,[tabindex]:not([tabindex="-1"]),[contenteditable="true"],[role="button"],[role="link"],[role="menuitem"]',
    );
    const menu = document.querySelector('[role="menu"][aria-label="Add songs"]');
    const trigger = document.querySelector('[data-slot="playlist-add-songs-button"]');
    const placement = menu?.contains(element)
      ? 'add-songs-menu'
      : menu?.parentElement === element.parentElement && element.dataset.type === 'inside'
        ? 'add-songs-menu'
        : trigger?.parentElement === element.parentElement
          ? 'add-songs-trigger'
          : null;
    const traceWindow = window as Window & {
      __baseUiMenuFocusTrace?: {
        ids: WeakMap<Element, string>;
        nextId: number;
      };
    };
    let traceIdentity = traceWindow.__baseUiMenuFocusTrace?.ids.get(element) ?? null;
    if (!traceIdentity && traceWindow.__baseUiMenuFocusTrace) {
      // Identity bookkeeping is observational; do not alter the guard DOM or move focus here.
      traceWindow.__baseUiMenuFocusTrace.nextId += 1;
      traceIdentity = `focus-${traceWindow.__baseUiMenuFocusTrace.nextId}`;
      traceWindow.__baseUiMenuFocusTrace.ids.set(element, traceIdentity);
    }
    const ancestors: string[] = [];
    for (
      let ancestor: HTMLElement | null = element.parentElement;
      ancestor;
      ancestor = ancestor.parentElement
    ) {
      ancestors.push(
        `${ancestor.tagName.toLowerCase()}${ancestor.id ? `#${ancestor.id}` : ''}${ancestor.dataset.slot ? `[data-slot="${ancestor.dataset.slot}"]` : ''}${ancestor.getAttribute('role') ? `[role="${ancestor.getAttribute('role')}"]` : ''}`,
      );
      if (ancestor === document.body) break;
    }
    return {
      placement,
      traceIdentity,
      context: {
        dataType: element.getAttribute('data-type'),
        sameTriggerParent: trigger?.parentElement === element.parentElement,
        insideAddSongsMenu: menu?.contains(element) ?? false,
        addSongsMenuOpen: menu !== null,
        triggerSlotPresent: trigger !== null,
        ancestors,
      },
      tagName: element.tagName,
      marker: element.hasAttribute('data-base-ui-focus-guard'),
      dataType: element.getAttribute('data-type'),
      ariaHidden: element.getAttribute('aria-hidden'),
      tabIndex: element.tabIndex,
      role: element.getAttribute('role'),
      text: element.textContent?.trim() ?? '',
      childElementCount: element.childElementCount,
      interactiveDescendantCount: interactiveDescendants.length,
      width: rect.width,
      height: rect.height,
      clipPath: style.clipPath,
      overflow: style.overflow,
      position: style.position,
      hiddenAncestor,
    };
  });
  const { placement, traceIdentity, ...shape } = measured;
  return {
    snapshotId,
    selector,
    matchCount,
    placement: placement as FocusGuardPlacement | null,
    traceIdentity,
    shape,
  };
}

let focusGuardSnapshotSequence = 0;

export async function readAddSongsMenuGuardSnapshot(page: Page) {
  focusGuardSnapshotSequence += 1;
  const snapshotId = `menu-snapshot-${focusGuardSnapshotSequence}`;
  const evidence = await Promise.all(
    REVIEWED_ADD_SONGS_GUARDS.map(({ selector }) =>
      readFocusGuardSelectorEvidence(page, selector, snapshotId),
    ),
  );
  return { snapshotId, evidence };
}

export async function beginFocusTraversalTrace(page: Page): Promise<string> {
  return page.evaluate(() => {
    type TraceEntry = {
      type: 'focusin' | 'focusout';
      id: string;
      timestamp: number;
      sentinel: boolean;
      guardPlacement: FocusGuardPlacement | null;
    };
    type TraceWindow = Window & {
      __baseUiMenuFocusTrace?: {
        ids: WeakMap<Element, string>;
        nextId: number;
        entries: TraceEntry[];
        startedAt: number;
      };
    };
    const traceWindow = window as TraceWindow;
    let trace = traceWindow.__baseUiMenuFocusTrace;
    if (!trace) {
      trace = {
        ids: new WeakMap<Element, string>(),
        nextId: 0,
        entries: [],
        startedAt: 0,
      };
      traceWindow.__baseUiMenuFocusTrace = trace;
    }
    if (!document.documentElement.dataset.baseUiMenuFocusTraceInstalled) {
      const record = (event: FocusEvent) => {
        if (!(event.target instanceof Element)) return;
        let id = trace.ids.get(event.target);
        if (!id) {
          trace.nextId += 1;
          id = `focus-${trace.nextId}`;
          trace.ids.set(event.target, id);
        }
        const sentinel = event.target.hasAttribute('data-base-ui-focus-guard');
        const menu = document.querySelector('[role="menu"][aria-label="Add songs"]');
        const trigger = document.querySelector('[data-slot="playlist-add-songs-button"]');
        const guardPlacement: FocusGuardPlacement | null = !sentinel
          ? null
          : menu?.contains(event.target) ||
              (menu?.parentElement === event.target.parentElement &&
                (event.target as HTMLElement).dataset.type === 'inside')
            ? 'add-songs-menu'
            : trigger?.parentElement === event.target.parentElement
              ? 'add-songs-trigger'
              : null;
        trace.entries.push({
          type: event.type as 'focusin' | 'focusout',
          id,
          timestamp: performance.now(),
          sentinel,
          guardPlacement,
        });
      };
      document.addEventListener('focusin', record, true);
      document.addEventListener('focusout', record, true);
      document.documentElement.dataset.baseUiMenuFocusTraceInstalled = 'true';
    }
    trace.entries = [];
    trace.startedAt = performance.now();
    const active = document.activeElement;
    if (!active) return 'no-active-element';
    let id = trace.ids.get(active);
    if (!id) {
      trace.nextId += 1;
      id = `focus-${trace.nextId}`;
      trace.ids.set(active, id);
    }
    return id;
  });
}

export async function finishFocusTraversalTrace(
  page: Page,
  key: 'Tab' | 'Shift+Tab',
  originId: string,
  menuClosed: boolean,
  snapshotId: string,
): Promise<FocusTraversalEvidence> {
  return page.evaluate(
    ({ key, originId, menuClosed, snapshotId }) => {
      type TraceEntry = {
        type: 'focusin' | 'focusout';
        id: string;
        timestamp: number;
        sentinel: boolean;
        guardPlacement: FocusGuardPlacement | null;
      };
      type TraceWindow = Window & {
        __baseUiMenuFocusTrace?: {
          ids: WeakMap<Element, string>;
          nextId: number;
          entries: TraceEntry[];
          startedAt: number;
        };
      };
      const trace = (window as TraceWindow).__baseUiMenuFocusTrace;
      const active = document.activeElement;
      if (!trace || !active) {
        return {
          key,
          snapshotId,
          originId,
          startedAt: 0,
          menuClosed,
          events: [],
          destination: {
            id: 'no-active-element',
            body: true,
            hidden: true,
            disabled: true,
            sentinel: false,
            visible: false,
            timestamp: performance.now(),
            logicalTarget: 'other',
            role: null,
            dataSlot: null,
            accessibleName: '',
          },
        };
      }
      let id = trace.ids.get(active);
      if (!id) {
        trace.nextId += 1;
        id = `focus-${trace.nextId}`;
        trace.ids.set(active, id);
      }
      const element = active instanceof HTMLElement ? active : null;
      const style = element ? getComputedStyle(element) : null;
      const hidden = (() => {
        for (let current = element; current; current = current.parentElement) {
          const currentStyle = getComputedStyle(current);
          if (
            current.hidden ||
            current.inert ||
            current.getAttribute('aria-hidden') === 'true' ||
            currentStyle.display === 'none' ||
            currentStyle.visibility === 'hidden'
          ) {
            return true;
          }
        }
        return false;
      })();
      const rect = element?.getBoundingClientRect();
      const events = trace.entries;
      const labelledBy = element?.getAttribute('aria-labelledby');
      const labelledText = labelledBy
        ?.split(/\s+/)
        .map((labelId) => document.getElementById(labelId)?.textContent?.trim() ?? '')
        .filter(Boolean)
        .join(' ');
      const accessibleName =
        element?.getAttribute('aria-label')?.trim() ||
        labelledText ||
        (element instanceof HTMLInputElement ? element.labels?.[0]?.textContent?.trim() : '') ||
        element?.getAttribute('title')?.trim() ||
        element?.textContent?.replace(/\s+/g, ' ').trim() ||
        '';
      const logicalTarget = element?.matches('[role="grid"]')
        ? 'track-grid'
        : element?.getAttribute('data-slot') === 'playlist-add-songs-button'
          ? 'playlist-add-trigger'
          : accessibleName === 'Browse Built-in Songs…' &&
              element?.closest('[data-slot="library-empty-state"]')
            ? 'empty-playlist-browse'
            : element?.closest('[data-slot="library-navigator"]')
              ? 'library-navigation'
              : 'other';
      return {
        key,
        snapshotId,
        originId,
        startedAt: trace.startedAt,
        menuClosed,
        events,
        destination: {
          id,
          body: active === document.body || active === document.documentElement,
          hidden,
          disabled:
            element instanceof HTMLButtonElement ||
            element instanceof HTMLInputElement ||
            element instanceof HTMLSelectElement ||
            element instanceof HTMLTextAreaElement
              ? element.disabled || element.getAttribute('aria-disabled') === 'true'
              : element?.getAttribute('aria-disabled') === 'true',
          sentinel: element?.hasAttribute('data-base-ui-focus-guard') ?? false,
          visible:
            element !== null &&
            element.getClientRects().length > 0 &&
            style?.display !== 'none' &&
            style?.visibility !== 'hidden' &&
            rect !== undefined &&
            rect.width > 0 &&
            rect.height > 0,
          timestamp: performance.now(),
          logicalTarget,
          role:
            element?.getAttribute('role') ??
            (element instanceof HTMLButtonElement ? 'button' : null),
          dataSlot: element?.dataset.slot ?? null,
          accessibleName,
        },
      };
    },
    { key, originId, menuClosed, snapshotId },
  );
}

export async function expectSafeKeyboardFocusDestination(page: Page): Promise<void> {
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const active = document.activeElement;
          if (!(active instanceof HTMLElement)) return false;
          if (
            active === document.body ||
            active === document.documentElement ||
            active.hasAttribute('data-base-ui-focus-guard') ||
            active.getAttribute('aria-disabled') === 'true'
          ) {
            return false;
          }
          for (
            let ancestor: HTMLElement | null = active;
            ancestor;
            ancestor = ancestor.parentElement
          ) {
            const style = getComputedStyle(ancestor);
            if (
              ancestor.hidden ||
              ancestor.inert ||
              ancestor.getAttribute('aria-hidden') === 'true' ||
              style.display === 'none' ||
              style.visibility === 'hidden'
            ) {
              return false;
            }
          }
          const style = getComputedStyle(active);
          const rect = active.getBoundingClientRect();
          const disabled =
            active instanceof HTMLButtonElement ||
            active instanceof HTMLInputElement ||
            active instanceof HTMLSelectElement ||
            active instanceof HTMLTextAreaElement
              ? active.disabled
              : false;
          return (
            !disabled &&
            style.display !== 'none' &&
            style.visibility !== 'hidden' &&
            active.getClientRects().length > 0 &&
            rect.width > 0 &&
            rect.height > 0
          );
        }),
      { timeout: 1000 },
    )
    .toBe(true);
}

export async function expectAddSongsMenuAccessibility(
  page: Page,
  testInfo: TestInfo,
  traversals: FocusTraversalEvidence[],
  guardSnapshots: Array<{ snapshotId: string; evidence: FocusGuardSelectorEvidence[] }>,
) {
  expect(reviewedAxeVersions.baseUi).toBe(REVIEWED_BASE_UI_VERSION);
  expect(reviewedAxeVersions.axe).toBe(REVIEWED_AXE_VERSION);
  // This is deliberately a full, unfiltered Axe run. The raw result is attached before review.
  const results = await new AxeBuilder({ page }).analyze();
  const rawReport = JSON.stringify(results, null, 2);
  await testInfo.attach('axe-full-unfiltered-add-songs.json', {
    body: rawReport,
    contentType: 'application/json',
  });
  const rawReportPath = testInfo.outputPath('axe-full-unfiltered-add-songs.json');
  mkdirSync(dirname(rawReportPath), { recursive: true });
  writeFileSync(rawReportPath, rawReport, 'utf8');
  const serious = results.violations.filter(
    (violation) => violation.impact === 'critical' || violation.impact === 'serious',
  ) as AxeViolation[];
  const blocking: AxeViolation[] = [];
  const adjudicated: Array<{ id: string; selectors: string[]; reason: string }> = [];
  const observations: Array<{
    id: string;
    selectors: string[];
    evidence: FocusGuardSelectorEvidence[];
    result: { adjudicated: boolean; reason: string };
  }> = [];
  focusGuardSnapshotSequence += 1;
  const axeSnapshotId = `axe-snapshot-${focusGuardSnapshotSequence}`;
  for (const violation of serious) {
    const selectors = selectorsForViolation(violation);
    const evidence =
      violation.id === 'aria-hidden-focus'
        ? await Promise.all(
            selectors.map((selector) =>
              readFocusGuardSelectorEvidence(page, selector, axeSnapshotId),
            ),
          )
        : [];
    const result = classifyBaseUiFocusGuardFinding(
      violation,
      [
        ...guardSnapshots.flatMap(({ evidence: snapshotEvidence }) => snapshotEvidence),
        ...evidence,
      ],
      traversals,
      reviewedAxeVersions,
    );
    observations.push({ id: violation.id, selectors, evidence, result });
    if (!result.adjudicated) blocking.push(violation);
    else {
      adjudicated.push({
        id: violation.id,
        selectors,
        placements: evidence.map(({ placement }) => placement),
        reason: result.reason,
      });
    }
  }
  if (adjudicated.length > 0) {
    testInfo.annotations.push({
      type: 'D11 Base UI 1.9.0 focus sentinel adjudication',
      description: JSON.stringify(adjudicated),
    });
  }
  const adjudicationEvidence = JSON.stringify(
    {
      versions: reviewedAxeVersions,
      reviewedGuards: REVIEWED_ADD_SONGS_GUARDS,
      guardSnapshots: guardSnapshots.map(({ snapshotId, evidence }) => ({ snapshotId, evidence })),
      observations,
      traversals,
    },
    null,
    2,
  );
  await testInfo.attach('axe-focus-guard-adjudication.json', {
    body: adjudicationEvidence,
    contentType: 'application/json',
  });
  const adjudicationEvidencePath = testInfo.outputPath('axe-focus-guard-adjudication.json');
  mkdirSync(dirname(adjudicationEvidencePath), { recursive: true });
  writeFileSync(adjudicationEvidencePath, adjudicationEvidence, 'utf8');
  expect(
    blocking,
    `Serious or critical unadjudicated Axe violations remain: ${JSON.stringify({ violations: blocking.map(({ id, nodes }) => ({ id, nodes })), observations })}`,
  ).toEqual([]);
}

export async function expectWorkbenchWithinBounds(
  page: Page,
  minimumTrackWidth = 420,
  minimumUtilityWidth = 280,
) {
  const overlay = page.locator('[data-pane="inspector"][data-presentation-mode="overlay"]');
  if (await overlay.count()) {
    await expect
      .poll(() => overlay.evaluate((element) => getComputedStyle(element).translate))
      .toBe('none');
  }
  const geometry = await page.evaluate(() => {
    const box = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return { x: rect.x, y: rect.y, right: rect.right, bottom: rect.bottom, width: rect.width };
    };
    return {
      viewportWidth: window.innerWidth,
      presentation:
        document.querySelector<HTMLElement>('[data-slot="workbench"]')?.dataset.utilityPresentation,
      gutter:
        document
          .querySelector<HTMLElement>('[data-slot="resizable-separator"]')
          ?.getBoundingClientRect().width ?? Number.NaN,
      solverGutter: Number.parseFloat(
        document.querySelector<HTMLElement>('[data-slot="workbench"]')?.dataset.workbenchGap ??
          'NaN',
      ),
      workbench: box('[data-slot="workbench"]'),
      navigator: box('[data-pane="navigator"]'),
      track: box('[data-pane="browser"]'),
      utility: box('[data-pane="inspector"]'),
      player: box('[data-slot="player-dock"]'),
      rootMinWidth: getComputedStyle(document.documentElement).minWidth,
      bodyMinWidth: getComputedStyle(document.body).minWidth,
    };
  });

  expect(geometry.workbench).not.toBeNull();
  expect(geometry.navigator).not.toBeNull();
  expect(geometry.track).not.toBeNull();
  if (geometry.workbench && geometry.navigator && geometry.track) {
    expect(geometry.solverGutter).toBe(geometry.gutter);
    expect(geometry.navigator.x).toBeGreaterThanOrEqual(geometry.workbench.x);
    expect(geometry.navigator.x + geometry.navigator.width).toBeLessThanOrEqual(geometry.track.x);
    expect(geometry.track.x + geometry.track.width).toBeLessThanOrEqual(
      geometry.workbench.x + geometry.workbench.width - geometry.gutter,
    );
    expect(geometry.track.width).toBeGreaterThanOrEqual(minimumTrackWidth);
  }
  if (geometry.utility && geometry.workbench) {
    expect(geometry.utility.x).toBeGreaterThanOrEqual(geometry.workbench.x);
    expect(geometry.utility.x + geometry.utility.width).toBeLessThanOrEqual(
      geometry.workbench.x + geometry.workbench.width - geometry.gutter,
    );
    if (geometry.presentation === 'docked') {
      if (geometry.track) {
        expect(geometry.track.x + geometry.track.width).toBeLessThanOrEqual(geometry.utility.x);
      }
      expect(geometry.utility.width).toBeGreaterThanOrEqual(minimumUtilityWidth);
      await expect(page.getByRole('separator', { name: 'Resize utility pane' })).toBeVisible();
    } else {
      expect(geometry.presentation).toBe('overlay');
      expect(geometry.utility.width).toBeGreaterThanOrEqual(280);
      expect(geometry.utility.width).toBeLessThanOrEqual(400);
      expect(geometry.player).not.toBeNull();
      if (geometry.player) {
        expect(geometry.utility.bottom).toBeLessThanOrEqual(geometry.player.y - geometry.gutter);
      }
      await expect(page.locator('[data-pane="inspector"]')).toHaveAttribute(
        'data-presentation-mode',
        'overlay',
      );
      await expect(page.getByRole('separator', { name: 'Resize utility pane' })).toHaveCount(0);
    }
    expect(Math.round(geometry.viewportWidth - geometry.utility.right)).toBeGreaterThanOrEqual(
      geometry.gutter,
    );
  } else {
    expect(geometry.presentation).toBe('closed');
  }
  expect(geometry.rootMinWidth).toBe('0px');
  expect(geometry.bodyMinWidth).toBe('0px');
  await expect(page.locator('[data-slot="workbench"]')).toHaveAttribute('data-layout-fits', 'true');
}

export async function readPlayerBarGeometry(page: Page) {
  return page.evaluate(() => {
    const player = document.querySelector<HTMLElement>('[data-slot="player-dock"]');
    const primary = player?.querySelector<HTMLElement>('[data-testid="player-primary-slot"]');
    const controls = player?.querySelector<HTMLElement>('[data-testid="player-controls-row"]');
    const timeline = player?.querySelector<HTMLElement>('[data-slot="player-timeline"]');
    const rail = player?.querySelector<HTMLElement>('[data-slot="player-timeline"] progress');
    if (!player || !primary || !controls || !timeline || !rail) return null;

    const rect = (element: Element) => {
      const box = element.getBoundingClientRect();
      return { x: box.x, y: box.y, width: box.width, height: box.height };
    };
    const controlSelectors: Record<string, string> = {
      Shuffle: 'button[aria-label="Shuffle"]',
      Previous: 'button[aria-label="Previous"]',
      Primary: '[data-slot="primary-transport"]',
      Next: 'button[aria-label="Next"]',
      Stop: 'button[aria-label="Stop"]',
    };
    const visibleControlCenters: Record<string, number> = {};
    const visibleControls: Record<
      string,
      { x: number; y: number; width: number; height: number; disabled: boolean; label: string }
    > = {};
    for (const [name, selector] of Object.entries(controlSelectors)) {
      const button = player.querySelector<HTMLButtonElement>(selector);
      if (button && button.getClientRects().length > 0) {
        const box = button.getBoundingClientRect();
        visibleControlCenters[name] = box.y + box.height / 2;
        visibleControls[name] = {
          x: box.x,
          y: box.y,
          width: box.width,
          height: box.height,
          disabled: button.disabled,
          label: button.getAttribute('aria-label') ?? '',
        };
      }
    }
    const primaryBox = rect(primary);
    return {
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
      viewportCenterX: window.innerWidth / 2,
      player: rect(player),
      controls: rect(controls),
      primary: primaryBox,
      primaryCenterX: primaryBox.x + primaryBox.width / 2,
      primaryCenterY: primaryBox.y + primaryBox.height / 2,
      timeline: rect(timeline),
      rail: rect(rail),
      visibleControlCenters,
      visibleControls,
      documentWidth: document.documentElement.scrollWidth,
      documentHeight: document.documentElement.scrollHeight,
    };
  });
}

export async function expectFiveTransportControls(player: Locator, primaryName: string) {
  for (const name of ['Shuffle', 'Previous', primaryName, 'Next', 'Stop']) {
    await expect(player.getByRole('button', { name, exact: true })).toBeVisible();
  }
  await expect(player.locator('[data-testid="player-controls-row"] button')).toHaveCount(5);
}

export async function readRightDockGeometry(page: Page) {
  return page.evaluate(() => {
    const selectors = {
      autoPlay: 'button[aria-label="Auto Play"]',
      profile: 'button[aria-label="Configure playback profile"]',
      utility: 'button[aria-label="Open utility panel"], button[aria-label="Close utility panel"]',
    };
    const result: Record<string, { x: number; right: number; centerY: number }> = {};
    for (const [name, selector] of Object.entries(selectors)) {
      const button = document.querySelector<HTMLElement>(`[data-slot="player-tools"] ${selector}`);
      if (!button) return null;
      const box = button.getBoundingClientRect();
      result[name] = { x: box.x, right: box.right, centerY: box.y + box.height / 2 };
    }
    return result;
  });
}

export function expectPlayerBarGeometry(
  geometry: Awaited<ReturnType<typeof readPlayerBarGeometry>>,
) {
  expect(geometry).not.toBeNull();
  if (!geometry) throw new Error('Player Bar geometry is unavailable');
  expect(Math.abs(geometry.primaryCenterX - geometry.viewportCenterX)).toBeLessThanOrEqual(2);
  expect(geometry.player.height).toBe(92);
  expect(
    geometry.timeline.y - (geometry.primary.y + geometry.primary.height),
  ).toBeGreaterThanOrEqual(6);
  expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.viewportWidth);
  expect(geometry.documentHeight).toBeLessThanOrEqual(geometry.viewportHeight);
  expect(Object.keys(geometry.visibleControls)).toHaveLength(5);
  for (const centerY of Object.values(geometry.visibleControlCenters)) {
    expect(Math.abs(centerY - geometry.primaryCenterY)).toBeLessThanOrEqual(1);
  }
}

export function expectStablePlayerBar(
  before: Awaited<ReturnType<typeof readPlayerBarGeometry>>,
  after: Awaited<ReturnType<typeof readPlayerBarGeometry>>,
) {
  expect(before).not.toBeNull();
  expect(after).not.toBeNull();
  if (!before || !after) throw new Error('Player Bar geometry is unavailable');
  expect(Math.abs(after.primaryCenterX - before.primaryCenterX)).toBeLessThanOrEqual(1);
  expect(Math.abs(after.primaryCenterY - before.primaryCenterY)).toBeLessThanOrEqual(1);
  expect(Math.abs(after.timeline.y - before.timeline.y)).toBeLessThanOrEqual(1);
  expect(Math.abs(after.rail.y - before.rail.y)).toBeLessThanOrEqual(1);
  expect(after.player.height).toBe(before.player.height);
}

export async function expectRecoveryBannerWithinViewport(page: Page) {
  const banner = page.getByRole('alert');
  await expect(banner).toBeVisible({ timeout: 20_000 });
  const box = await banner.boundingBox();
  const viewport = page.viewportSize();
  expect(box).not.toBeNull();
  expect(viewport).not.toBeNull();
  if (box && viewport) {
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(viewport.width);
    expect(box.y + box.height).toBeLessThanOrEqual(viewport.height);
  }
  const documentWidth = await page.evaluate(() => document.documentElement.scrollWidth);
  expect(documentWidth).toBeLessThanOrEqual(viewport?.width ?? 0);
}

export async function startSelectedSong(
  page: Page,
  options: { waitFor?: 'playing' | 'pending' | 'recovery' } = {},
) {
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const proceed = player.getByRole('button', { name: 'Proceed with current settings' });
  if (options.waitFor === 'recovery') {
    await expect(page.getByRole('alert')).toBeVisible();
  } else if (options.waitFor === 'pending') {
    await expect(player.getByRole('button', { name: 'Starting playback' })).toBeDisabled();
  } else {
    if (await proceed.isVisible().catch(() => false)) await proceed.click();
    await expect(player.locator('[data-slot="player-track-status"]')).toHaveText('Playing');
  }
  return player;
}

export async function chooseSelectOption(page: Page, trigger: Locator, value: string) {
  await trigger.click();
  await page.locator(`[data-slot="select-item"][data-value="${value}"]`).click();
  await expect(page.locator('[data-slot="select-content"]:visible')).toHaveCount(0);
}

export async function selectPalette(page: Page, palette: PaletteId) {
  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await dialog.getByRole('button', { name: 'Appearance', exact: true }).click();
  await chooseSelectOption(page, dialog.getByRole('combobox', { name: 'Dark palette' }), palette);
  await expect(page.locator('html')).toHaveAttribute('data-palette', palette);
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
  await dialog.getByRole('button', { name: 'Close settings' }).click();
  await expect(dialog).toBeHidden();
}
