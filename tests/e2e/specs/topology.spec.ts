import { test, expect } from '../fixtures/test';
import { SESSION_ID, BINARY } from '../constants';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve, join } from 'node:path';

test.describe('topology tile', () => {
  test('disjoint direct-service loops stack with the same right edge', async ({ page, e2eState }) => {
    const env = { ...process.env, XDG_DATA_HOME: join(e2eState.tmpDir, 'data'), XDG_STATE_HOME: join(e2eState.tmpDir, 'state') };
    const db = join(e2eState.tmpDir, 'stacked-loops.duckdb');
    const run = (args: string[]) => execFileSync(BINARY, ['--port', String(e2eState.port), ...args], { env });
    run([db, '-c', `CREATE TABLE links AS SELECT 'Port ' || i AS src, 'MVE ' || ((i+1)//2) AS dst, 'DC-1' AS dc, '10G' AS label FROM range(1,7) t(i);`]);
    run(['session', 'tile', SESSION_ID, '--name', 'stacked-loops', '--db', db, '--view', 'links',
      '--chart', 'topology', '--from', 'src', '--to', 'dst', '--from-within', 'dc', '--to-within', 'dc',
      '--label', 'label', '--caption', 'Independent direct connections share one routing lane.']);
    await page.goto(`/session/${SESSION_ID}/`);
    const tracks = page.locator('.panel[data-tile="stacked-loops"] .topo-track');
    await expect(tracks).toHaveCount(6);
    const rightEdges = await tracks.evaluateAll((paths) => paths.map((path) => {
      const box = (path as SVGPathElement).getBBox();
      return box.x + box.width;
    }));
    expect(Math.max(...rightEdges) - Math.min(...rightEdges)).toBeLessThan(1);
  });

  test('mesh labels clear all tracks, stay inside the diagram, and highlight their connection', async ({ page, e2eState }) => {
    const env = { ...process.env, XDG_DATA_HOME: join(e2eState.tmpDir, 'data'), XDG_STATE_HOME: join(e2eState.tmpDir, 'state') };
    const db = join(e2eState.tmpDir, 'router-mesh.duckdb');
    const run = (args: string[], input?: string) => execFileSync(BINARY, ['--port', String(e2eState.port), ...args], { env, input, stdio: ['pipe', 'pipe', 'pipe'] });
    run([db], readFileSync(resolve(__dirname, '../fixtures/router-mesh.sql'), 'utf8'));
    run(['session', 'tile', SESSION_ID, '--name', 'router-mesh', '--db', db, '--view', 'router_mesh',
      '--chart', 'topology', '--from', 'src', '--to', 'dst', '--from-label', 'src_name', '--to-label', 'dst_name',
      '--from-within', 'src_dc', '--to-within', 'dst_dc', '--from-port', 'src_ip', '--to-port', 'dst_ip',
      '--label', 'subnet', '--color', 'link_class', '--caption', 'Full mesh regression fixture.']);
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="router-mesh"]');
    await expect(panel.locator('.topo-track')).toHaveCount(8);
    await expect(panel.locator('.topo-node')).toHaveCount(6);
    const failures = await panel.locator('.topo-svg').evaluate((element) => {
      const svg = element as SVGSVGElement, vb = svg.viewBox.baseVal;
      const tracks = [...svg.querySelectorAll<SVGPathElement>('.topo-track')];
      const failures: string[] = [];
      for (const label of svg.querySelectorAll<SVGGraphicsElement>('.topo-edge-label, .topo-port-label')) {
        const b = label.getBBox();
        if (b.x < vb.x || b.y < vb.y || b.x + b.width > vb.x + vb.width || b.y + b.height > vb.y + vb.height) failures.push(`clipped: ${label.textContent}`);
        for (const path of tracks) {
          for (let at = 0; at <= path.getTotalLength(); at += 2) {
            const p = path.getPointAtLength(at);
            if (p.x > b.x - 2 && p.x < b.x + b.width + 2 && p.y > b.y - 2 && p.y < b.y + b.height + 2) {
              failures.push(`covered track: ${label.textContent}`); break;
            }
          }
        }
      }
      const nodes = [...svg.querySelectorAll<SVGRectElement>('.topo-node')];
      if (new Set(nodes.map((n) => n.x.baseVal.value)).size !== 1) failures.push('default peers are not vertically stacked');
      return failures;
    });
    expect(failures).toEqual([]);
    // Close parallel tracks must not strand a title beside somebody else's
    // connection. Slide it along its own trunk to retain a short attachment.
    const leaderLengths = await panel.locator('.topo-label-leader').evaluateAll((leaders) =>
      leaders.map((leader) => (leader as SVGPathElement).getTotalLength()));
    expect(leaderLengths.every((length) => length <= 8)).toBe(true);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    const label = panel.locator('.topo-edge-label-pill').first();
    const edge = await label.getAttribute('data-topo-edge');
    const track = panel.locator(`.topo-track[data-topo-edge="${edge}"]`);
    await expect(label.locator('.topo-label-leader')).toHaveCount(1);
    const attachmentDistance = await label.locator('.topo-label-anchor').evaluate((anchor) => {
      const circle = anchor as SVGCircleElement;
      const edge = anchor.parentElement!.getAttribute('data-topo-edge');
      const path = anchor.closest('svg')!.querySelector<SVGPathElement>(`.topo-track[data-topo-edge="${edge}"]`)!;
      let distance = Infinity;
      for (let at = 0; at <= path.getTotalLength(); at += 1) {
        const p = path.getPointAtLength(at);
        distance = Math.min(distance, Math.hypot(p.x - circle.cx.baseVal.value, p.y - circle.cy.baseVal.value));
      }
      return distance;
    });
    expect(attachmentDistance).toBeLessThan(1);
    await label.locator('.topo-edge-label').hover();
    await expect(track).toHaveCSS('stroke-width', '4.5px');
    await expect(label.locator('.topo-edge-label-bg')).toHaveCSS('filter', 'brightness(1.4)');
    await expect(label.locator('.topo-edge-label')).toHaveCSS('text-decoration-line', 'none');
    await page.mouse.move(0, 0);
    await expect(track).toHaveCSS('stroke-width', '2.4px');
    await track.scrollIntoViewIfNeeded();
    const point = await track.evaluate((el) => {
      const path = el as SVGPathElement, p = path.getPointAtLength(path.getTotalLength() / 2);
      const transformed = new DOMPoint(p.x, p.y).matrixTransform(path.getScreenCTM()!);
      return { x: transformed.x, y: transformed.y };
    });
    await page.mouse.move(point.x, point.y);
    await expect(label.locator('.topo-edge-label-bg')).toHaveCSS('filter', 'brightness(1.4)');
    await expect(label.locator('.topo-edge-label')).toHaveCSS('text-decoration-line', 'none');
  });

  test('four MCRs and four AWS LAGs retain labels next to their ports', async ({ page, e2eState }) => {
    const env = { ...process.env, XDG_DATA_HOME: join(e2eState.tmpDir, 'data'), XDG_STATE_HOME: join(e2eState.tmpDir, 'state') };
    const db = join(e2eState.tmpDir, 'aws-mesh.duckdb');
    const run = (args: string[], input?: string) => execFileSync(BINARY, ['--port', String(e2eState.port), ...args], { env, input, stdio: ['pipe', 'pipe', 'pipe'] });
    run([db], `
      CREATE TABLE nodes AS
      SELECT 'mcr-' || i AS id, 'MCR ' || i AS name, 'Customer' AS domain, 'Core' AS grp, 'MCR mesh' AS zone FROM range(1,5) t(i)
      UNION ALL SELECT 'lag-' || i, 'LAG ' || i, 'Customer', 'Edge', 'AWS LAGs' FROM range(1,5) t(i)
      UNION ALL SELECT 'aws', 'AWS Direct Connect', 'Provider', 'AWS', 'ap-southeast-2';
      CREATE TABLE links AS
      SELECT a.id AS src, b.id AS dst, 'iBGP' AS label, 'mesh-' || right(b.id,1) AS src_port, 'mesh-' || right(a.id,1) AS dst_port, 'mesh' AS kind
      FROM nodes a, nodes b WHERE a.id LIKE 'mcr-%' AND b.id LIKE 'mcr-%' AND a.id < b.id
      UNION ALL SELECT 'mcr-' || i, 'lag-' || i, 'LACP', 'ae' || i, 'bundle', 'aggregation' FROM range(1,5) t(i)
      UNION ALL SELECT 'lag-' || i, 'aws', '100G DX', 'member-' || i, 'dx-' || i, 'direct-connect' FROM range(1,5) t(i);
      CREATE VIEW aws_mesh AS SELECT l.*, a.name src_name, b.name dst_name,
        a.domain src_domain, a.grp src_group, a.zone src_zone,
        b.domain dst_domain, b.grp dst_group, b.zone dst_zone
      FROM links l JOIN nodes a ON a.id=l.src JOIN nodes b ON b.id=l.dst ORDER BY l.src, l.dst;`);
    run(['session', 'tile', SESSION_ID, '--name', 'aws-mesh', '--db', db, '--view', 'aws_mesh',
      '--chart', 'topology', '--from', 'src', '--to', 'dst', '--from-label', 'src_name', '--to-label', 'dst_name',
      '--from-within', 'src_domain,src_group,src_zone', '--to-within', 'dst_domain,dst_group,dst_zone',
      '--from-port', 'src_port', '--to-port', 'dst_port', '--label', 'label', '--color', 'kind',
      '--caption', 'Four MCR mesh nodes through four LAGs into AWS.']);
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="aws-mesh"]');
    await expect(panel.locator('.topo-track')).toHaveCount(14);
    const displaced = await panel.locator('.topo-svg').evaluate((svg) => {
      const failed: string[] = [];
      for (const path of svg.querySelectorAll<SVGPathElement>('.topo-track')) {
        const ports = [...svg.querySelectorAll<SVGTextElement>(`.topo-label-layer [data-topo-edge="${path.dataset.topoEdge}"] .topo-port-label`)];
        ports.forEach((port, i) => {
          const p = path.getPointAtLength(i ? path.getTotalLength() : 0);
          if (Math.abs(port.x.baseVal[0].value - p.x) > 35 || Math.abs(port.y.baseVal[0].value - p.y) > 30) failed.push(port.textContent || 'port');
        });
      }
      return failed;
    });
    expect(displaced).toEqual([]);
    const escaped = await panel.locator('.topo-svg').evaluate((svg) => {
      const zones = [...svg.querySelectorAll<SVGTextElement>('.topo-zone-label')];
      const failed: string[] = [];
      for (const path of svg.querySelectorAll<SVGPathElement>('.topo-track')) {
        const title = path.parentElement!.querySelector('title')!.textContent || '';
        const container = title.includes('iBGP') ? 'MCR mesh' : title.includes('LACP') ? 'Customer' : null;
        if (!container) continue;
        const zone = zones.find((z) => z.textContent === container)!.parentElement!.querySelector<SVGRectElement>('rect')!.getBBox();
        for (let at = 0; at <= path.getTotalLength(); at += 2) {
          const p = path.getPointAtLength(at);
          if (p.x < zone.x || p.x > zone.x + zone.width || p.y < zone.y || p.y > zone.y + zone.height) {
            failed.push(title); break;
          }
        }
      }
      return failed;
    });
    expect(escaped).toEqual([]);
    // With four vertically stacked peers and links on one side, only the
    // interleaving 1→3 / 2→4 pair needs to cross. Shared endpoints and nested
    // spans must not manufacture additional crossings.
    const crossings = await panel.locator('.topo-svg').evaluate((svg) => {
      const routes = [...svg.querySelectorAll<SVGPathElement>('.topo-track')]
        .filter((path) => path.parentElement!.querySelector('title')!.textContent!.includes('iBGP'))
        .map((path) => {
          const a = path.getPointAtLength(0), b = path.getPointAtLength(path.getTotalLength());
          return { y1: Math.min(a.y, b.y), y2: Math.max(a.y, b.y), x: path.getBBox().x + path.getBBox().width };
        });
      let n = 0;
      for (let i = 0; i < routes.length; i++) for (let j = i + 1; j < routes.length; j++) {
        const [inner, outer] = routes[i].x < routes[j].x ? [routes[i], routes[j]] : [routes[j], routes[i]];
        n += + (outer.y1 > inner.y1 && outer.y1 < inner.y2);
        n += + (outer.y2 > inner.y1 && outer.y2 < inner.y2);
      }
      return n;
    });
    expect(crossings).toBe(1);
  });

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
