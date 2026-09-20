import { test, expect } from '../fixtures/test';
import { SESSION_ID } from '../constants';

test.describe('topology tile', () => {
  test('renders nested containment, typed nodes, labeled tracks and markup badges', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="topology"]');
    await expect(panel).toBeVisible();

    await expect(panel.locator('.topo-node-wrap')).toHaveCount(7);
    await expect(panel.locator('.topo-track')).toHaveCount(5);
    await expect(panel.locator('.topo-zone')).toHaveCount(7);
    await expect(panel.locator('.topo-edge-label').filter({ hasText: 'HTTPS' })).toHaveCount(2);
    await expect(panel.locator('.topo-edge-label-pill')).toHaveCount(2);
    await expect(panel.locator('.topo-edge-label-pill .topo-edge-label').filter({ hasText: 'HTTPS' })).toBeVisible();
    await expect(panel.locator('.topo-port-label').filter({ hasText: '8443' })).toHaveCount(2);
    await expect(panel.locator('.topo-mark').filter({ hasText: 'az: az-b' }).first()).toBeVisible();
    await expect(panel.locator('.topo-type').filter({ hasText: 'application · service' })).toBeVisible();

    // Service cards expand for long names instead of clipping their contents,
    // and the surrounding rank layout uses that measured width.
    const nodeWidths = await panel.locator('.topo-node-wrap').evaluateAll((wraps) => {
      const width = (id: string) => {
        const wrap = wraps.find((el) => el.getAttribute('data-node') === id)!;
        return +(wrap.querySelector('.topo-node') as SVGRectElement).getAttribute('width')!;
      };
      return [width('edge-a'), width('api-a')];
    });
    expect(nodeWidths[1]).toBeGreaterThan(nodeWidths[0]);

    // Same leaf names under distinct ancestry remain separate enclosures, and
    // the isolated observer still renders without manufacturing an edge.
    await expect(panel.locator('.topo-zone-label').filter({ hasText: 'dc-1' })).toHaveCount(1);
    await expect(panel.locator('.topo-node-wrap[data-node="observer"]')).toBeVisible();

    // Sibling containment zones are packed into separate bands. In particular,
    // dc-2 must never be drawn inside or overlapping dc-1.
    const dcBoxes = await panel.locator('.topo-zone-label').evaluateAll((labels) => {
      const box = (name: string) => {
        const label = labels.find((el) => el.textContent === name)!;
        const rect = label.parentElement!.querySelector('rect') as SVGRectElement;
        return { x: +rect.getAttribute('x')!, y: +rect.getAttribute('y')!,
          w: +rect.getAttribute('width')!, h: +rect.getAttribute('height')! };
      };
      return [box('dc-1'), box('dc-2')];
    });
    const [dc1, dc2] = dcBoxes;
    const intersects = dc1.x < dc2.x + dc2.w && dc1.x + dc1.w > dc2.x
      && dc1.y < dc2.y + dc2.h && dc1.y + dc1.h > dc2.y;
    expect(intersects).toBe(false);
    const metroOrigins = await panel.locator('.topo-zone-label').evaluateAll((labels) =>
      ['brisbane', 'sydney'].map((name) => {
        const label = labels.find((el) => el.textContent?.toLowerCase() === name)!;
        return +(label.parentElement!.querySelector('rect') as SVGRectElement).getAttribute('x')!;
      }));
    expect(metroOrigins[0]).toBe(metroOrigins[1]);

    // Routing lanes encode crossing scope: the DC1→Sydney metro crossing is
    // farther right than the DC2→DC1 crossing, regardless of row order.
    const trunkXs = await panel.locator('.topo-track').evaluateAll((tracks) => [3, 4].map((i) => {
      const nums = (tracks[i].getAttribute('d') || '').match(/-?\d+(?:\.\d+)?/g)?.map(Number) || [];
      return Math.max(...nums.filter((_, n) => n % 2 === 0));
    }));
    expect(trunkXs[1]).toBeGreaterThan(trunkXs[0]);

    // API's cross-DC arrival and cross-metro departure share its right side.
    // Their horizontal stubs must use distinct ports, as with LACP/DX at a LAG.
    const sharedSideYs = await panel.locator('.topo-track').evaluateAll((tracks) => {
      const numbers = (index: number) => (tracks[index].getAttribute('d') || '')
        .match(/-?\d+(?:\.\d+)?/g)!.map(Number);
      const arrival = numbers(3), departure = numbers(4);
      return [arrival[arrival.length - 1], departure[1]];
    });
    expect(Math.abs(sharedSideYs[0] - sharedSideYs[1])).toBeGreaterThanOrEqual(18);

    // Topologies can break out to the full viewport and use intrinsic SVG
    // height, so they deliberately have no drag-resize grip.
    await expect(panel.locator('[data-widen]')).toHaveCount(1);
    await expect(panel.locator('.panel-grip')).toHaveCount(0);
  });
});
