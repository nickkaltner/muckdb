import { test, expect } from '../fixtures/test';
import { SESSION_ID, BINARY } from '../constants';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve, join } from 'node:path';

test.describe('topology tile', () => {
  test('core fanout reserves a clear gutter before the second service column', async ({ page, e2eState }) => {
    const env = { ...process.env, XDG_DATA_HOME: join(e2eState.tmpDir, 'data'), XDG_STATE_HOME: join(e2eState.tmpDir, 'state') };
    const db = join(e2eState.tmpDir, 'core-fanout.duckdb');
    const run = (args: string[]) => execFileSync(BINARY, ['--port', String(e2eState.port), ...args], { env });
    run([db, '-c', `CREATE TABLE links AS
      SELECT 'Core switch' src, 'Port ' || i dst, 'et-0/0/' || i src_port, 'uplink' dst_port, '100G' AS label, 'DC-1' dc
      FROM range(1,7) t(i)
      UNION ALL SELECT 'Port ' || i, 'MVE ' || ((i+1)//2), 'access', 'ge-0/0/' || (1+(i+1)%2), '10G', 'DC-1'
      FROM range(1,7) t(i);`]);
    run(['session', 'tile', SESSION_ID, '--name', 'core-fanout', '--db', db, '--view', 'links',
      '--chart', 'topology', '--from', 'src', '--to', 'dst', '--from-within', 'dc', '--to-within', 'dc',
      '--from-port', 'src_port', '--to-port', 'dst_port', '--label', 'label', '--routing', 'metro',
      '--caption', 'Core switch through six ports to three MVEs.']);
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="core-fanout"]');
    for (const mode of ['Wide', '2 columns']) {
      await panel.getByRole('button', { name: mode, exact: true }).click();
      const failures = await panel.locator('.topo-svg').evaluate((svg) => {
        const nodes = [...svg.querySelectorAll<SVGRectElement>('.topo-node')].map((node) => node.getBBox());
        const failures: string[] = [];
        for (const path of svg.querySelectorAll<SVGPathElement>('.topo-track')) {
          for (let d = 2; d < path.getTotalLength(); d += 2) {
            const p = path.getPointAtLength(d);
            if (nodes.some((b) => p.x > b.x + 1 && p.x < b.x + b.width - 1 && p.y > b.y + 1 && p.y < b.y + b.height - 1)) {
              failures.push(path.dataset.topoEdge!); break;
            }
          }
        }
        const port = svg.querySelector<SVGRectElement>('[data-node="Port 5"] .topo-node')!.getBBox();
        const path = svg.querySelector<SVGPathElement>('.topo-track[data-topo-edge="4"]')!;
        const port6Path = svg.querySelector<SVGPathElement>('.topo-track[data-topo-edge="5"]')!;
        const verticalXs = (route: SVGPathElement) => {
          // The first rounded elbow's control point lies on the trunk,
          // including short routes whose two curves consume the straight run.
          const elbow = route.getAttribute('d')!.match(/Q(-?[\d.]+)/);
          return elbow ? [Number(elbow[1])] : [];
        };
        const port5Xs = verticalXs(path), port6Xs = verticalXs(port6Path);
        if (!port5Xs.length || !port6Xs.length || port5Xs.some((x) => port6Xs.some((other) => Math.abs(x - other) < 12))) {
          failures.push('Core switch to Ports 5 and 6 share a routing lane');
        }
        for (const [first, second] of [[6, 7], [8, 9]]) {
          const a = verticalXs(svg.querySelector<SVGPathElement>(`.topo-track[data-topo-edge="${first}"]`)!);
          const b = verticalXs(svg.querySelector<SVGPathElement>(`.topo-track[data-topo-edge="${second}"]`)!);
          if (!a.length || !b.length || a.some((x) => b.some((other) => Math.abs(x - other) < 12))) {
            failures.push(`MVE pair ${first}/${second} shares a routing lane`);
          }
        }
        for (let d = 2; d < path.getTotalLength() - 2; d += 2) {
          const p = path.getPointAtLength(d), next = path.getPointAtLength(d + 1);
          if (Math.abs(p.x - next.x) < .01 && Math.abs(p.y - next.y) > .9 && p.x > port.x - 24) {
            failures.push('Port 5 vertical trunk has insufficient clearance'); break;
          }
        }
        return failures;
      });
      expect(failures).toEqual([]);
    }
  });

  test('edge-to-data links do not share horizontal runs across columns', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="topology"]');
    for (const mode of ['Wide', '2 columns']) {
      await panel.getByRole('button', { name: mode, exact: true }).click();
      const overlap = await panel.locator('.topo-svg').evaluate((svg) => {
        const paths = [...svg.querySelectorAll<SVGPathElement>('.topo-track')];
        const edgeToLb = paths[0], lbToApi = paths[2];
        const samples = (path: SVGPathElement) => {
          const points = [];
          for (let d = 1; d < path.getTotalLength(); d += 1) points.push(path.getPointAtLength(d));
          return points;
        };
        const a = samples(edgeToLb), b = samples(lbToApi);
        // A crossing is a point; an overlapping run has many adjacent samples.
        let longest = 0, run = 0;
        for (const p of b) {
          run = a.some((q) => Math.hypot(p.x - q.x, p.y - q.y) < 2) ? run + 1 : 0;
          longest = Math.max(longest, run);
        }
        return longest;
      });
      expect(overlap).toBeLessThan(8);
    }
  });

  test('wide layout separates local services and packs independent areas, with persistent per-tile controls', async ({ page, e2eState }) => {
    const env = { ...process.env, XDG_DATA_HOME: join(e2eState.tmpDir, 'data'), XDG_STATE_HOME: join(e2eState.tmpDir, 'state') };
    const db = join(e2eState.tmpDir, 'wide.duckdb');
    const run = (args: string[]) => execFileSync(BINARY, ['--port', String(e2eState.port), ...args], { env });
    run([db, '-c', `CREATE TABLE links AS SELECT * FROM (VALUES
      ('local-a','gateway','DC-1','DC-1'), ('local-b','gateway','DC-1','DC-1'),
      ('gateway','remote','DC-1','DC-2'), ('other-a','other-b','DC-3','DC-3')) t(src,dst,src_dc,dst_dc);`]);
    run(['session', 'tile', SESSION_ID, '--name', 'wide-layout', '--db', db, '--view', 'links',
      '--chart', 'topology', '--from', 'src', '--to', 'dst', '--from-within', 'src_dc', '--to-within', 'dst_dc',
      '--caption', 'Local services on the left; independent areas side by side.']);
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="wide-layout"]');
    await expect(panel.getByRole('button', { name: 'Wide', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await expect(panel.locator('.topo-svg')).toHaveCount(2);
    const boxes = await panel.locator('.topo-svg').evaluateAll((svgs) => svgs.map((svg) => {
      const box = svg.getBoundingClientRect(); return { x: box.x, y: box.y };
    }));
    const gridStyle = await panel.locator('.topo-grid').evaluate((el) => ({ display: getComputedStyle(el).display, columns: getComputedStyle(el).gridTemplateColumns, width: el.getBoundingClientRect().width }));
    expect(boxes[0].y, JSON.stringify(gridStyle)).toBe(boxes[1].y);
    expect(boxes[0].x).toBeLessThan(boxes[1].x);
    const positions = () => panel.locator('.topo-node-wrap').evaluateAll((nodes) => Object.fromEntries(nodes.map((n) =>
      [n.getAttribute('data-node'), +(n.querySelector('rect')!).getAttribute('x')!])));
    let x = await positions();
    expect(x['local-a']).toBeLessThan(x.gateway);
    expect(x['local-b']).toBeLessThan(x.gateway);
    await panel.getByRole('button', { name: '2 columns', exact: true }).click();
    await expect(panel.locator('.topo-svg')).toHaveCount(1);
    await panel.getByRole('button', { name: '1 column', exact: true }).click();
    x = await positions();
    expect(new Set(Object.values(x)).size).toBe(1);
    await page.reload();
    await expect(panel.getByRole('button', { name: '1 column', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await panel.getByRole('button', { name: 'Wide', exact: true }).click();
    await page.setViewportSize({ width: 1440, height: 1800 });
    await page.addStyleTag({ content: '.statusline { visibility: hidden; }' });
    await panel.evaluate((el) => el.scrollIntoView({ block: 'center' }));
    await panel.screenshot({ path: '/tmp/muckdb-topology-wide.png' });
  });

  test('disjoint direct-service loops stack with the same right edge', async ({ page, e2eState }) => {
    const env = { ...process.env, XDG_DATA_HOME: join(e2eState.tmpDir, 'data'), XDG_STATE_HOME: join(e2eState.tmpDir, 'state') };
    const db = join(e2eState.tmpDir, 'stacked-loops.duckdb');
    const run = (args: string[]) => execFileSync(BINARY, ['--port', String(e2eState.port), ...args], { env });
    run([db, '-c', `CREATE TABLE links AS SELECT 'Port ' || i AS src, 'MVE ' || ((i+1)//2) AS dst, 'DC-1' AS dc, '10G' AS label FROM range(1,7) t(i);`]);
    run(['session', 'tile', SESSION_ID, '--name', 'stacked-loops', '--db', db, '--view', 'links',
      '--chart', 'topology', '--from', 'src', '--to', 'dst', '--from-within', 'dc', '--to-within', 'dc',
      '--label', 'label', '--caption', 'Independent direct connections share one routing lane.']);
    await page.goto(`/session/${SESSION_ID}/`);
    await page.locator('.panel[data-tile="stacked-loops"]').getByRole('button', { name: '1 column', exact: true }).click();
    const tracks = page.locator('.panel[data-tile="stacked-loops"] .topo-track');
    await expect(tracks).toHaveCount(6);
    const rightEdges = await tracks.evaluateAll((paths) => paths.map((path) => {
      const box = (path as SVGPathElement).getBBox();
      return box.x + box.width;
    }));
    expect(Math.max(...rightEdges) - Math.min(...rightEdges)).toBeLessThan(1);
    for (const mode of ['2 columns', 'Wide']) {
      await page.locator('.panel[data-tile="stacked-loops"]').getByRole('button', { name: mode, exact: true }).click();
      const gap = await tracks.evaluateAll((paths) => {
        // Port 4 is in the second column, while MVE 2 is in the first.
        // Its long vertical segment must clear the Port 1/2/3 local loops.
        const right = (path: Element) => { const box = (path as SVGPathElement).getBBox(); return box.x + box.width; };
        const path = paths[3] as SVGPathElement;
        const p = path.getPointAtLength(path.getTotalLength() / 2);
        return p.x - Math.max(...paths.slice(0, 3).map(right));
      });
      expect(gap).toBeGreaterThanOrEqual(12);
    }
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
    await panel.getByRole('button', { name: '1 column', exact: true }).click();
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
    await expect(label.locator('.topo-edge-label-bg')).toHaveCSS('filter', 'none');
    const stroke = await track.evaluate((el) => getComputedStyle(el).stroke);
    await expect(label.locator('.topo-edge-label-bg')).toHaveCSS('fill', stroke);
    await expect(track).not.toHaveAttribute('marker-end');
    await expect(label.locator('.topo-edge-label')).toHaveCSS('text-decoration-line', 'none');
    // Wider modes must keep every wire clear of service cards.
    for (const mode of ['2 columns', 'Wide']) {
      await panel.getByRole('button', { name: mode, exact: true }).click();
      const collisions = await panel.locator('.topo-svg').evaluate((svg) => {
        const boxes = [...svg.querySelectorAll<SVGRectElement>('.topo-node')].map((node) => node.getBBox());
        const collisions: string[] = [];
        for (const path of svg.querySelectorAll<SVGPathElement>('.topo-track')) {
          for (let d = 2; d < path.getTotalLength(); d += 2) {
            const p = path.getPointAtLength(d);
            if (boxes.some((b) => p.x > b.x + 1 && p.x < b.x + b.width - 1 && p.y > b.y + 1 && p.y < b.y + b.height - 1)) {
              collisions.push(path.dataset.topoEdge!); break;
            }
          }
        }
        return collisions;
      });
      expect(collisions).toEqual([]);
    }
    await page.mouse.move(0, 0);
    await expect(track).toHaveCSS('stroke-width', '2.4px');
    await track.scrollIntoViewIfNeeded();
    const point = await track.evaluate((el) => {
      const path = el as SVGPathElement, p = path.getPointAtLength(path.getTotalLength() / 2);
      const transformed = new DOMPoint(p.x, p.y).matrixTransform(path.getScreenCTM()!);
      return { x: transformed.x, y: transformed.y };
    });
    await page.mouse.move(point.x, point.y);
    await expect(label.locator('.topo-edge-label-bg')).toHaveCSS('filter', 'none');
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
    // Wide mode packs the connected areas horizontally, retaining one stack
    // within each area and a separate gutter for the internal MCR mesh.
    await expect(panel.getByRole('button', { name: 'Wide', exact: true })).toHaveAttribute('aria-pressed', 'true');
    const wideXs = await panel.locator('.topo-node').evaluateAll((nodes) => nodes.map((n) => +(n as SVGRectElement).getAttribute('x')!));
    expect(new Set(wideXs).size).toBe(3);
    const wideHeight = await panel.locator('.topo-svg').evaluate((svg) => (svg as SVGSVGElement).viewBox.baseVal.height);
    const hiddenTracks = await panel.locator('.topo-svg').evaluate((svg) => {
      const boxes = [...svg.querySelectorAll<SVGRectElement>('.topo-node')].map((node) => node.getBBox());
      return [...svg.querySelectorAll<SVGPathElement>('.topo-track')].filter((path) => {
        for (let d = 2; d < path.getTotalLength(); d += 2) {
          const p = path.getPointAtLength(d);
          if (boxes.some((b) => p.x > b.x + 1 && p.x < b.x + b.width - 1 && p.y > b.y + 1 && p.y < b.y + b.height - 1)) return true;
        }
        return false;
      }).length;
    });
    expect(hiddenTracks).toBe(0);
    const detachedLabels = await panel.locator('.topo-svg').evaluate((svg) => {
      return [...svg.querySelectorAll<SVGCircleElement>('.topo-label-anchor')].filter((anchor) => {
        const edge = anchor.parentElement!.getAttribute('data-topo-edge');
        const path = svg.querySelector<SVGPathElement>(`.topo-track[data-topo-edge="${edge}"]`)!;
        let distance = Infinity;
        for (let d = 0; d <= path.getTotalLength(); d += 1) {
          const p = path.getPointAtLength(d);
          distance = Math.min(distance, Math.hypot(p.x - anchor.cx.baseVal.value, p.y - anchor.cy.baseVal.value));
        }
        return distance > 1;
      }).length;
    });
    expect(detachedLabels).toBe(0);
    const lacpOffsets = await panel.locator('.topo-edge-label-pill').evaluateAll((labels) => labels
      .filter((label) => label.querySelector('.topo-edge-label')?.textContent === 'LACP')
      .map((label) => {
        const edge = label.getAttribute('data-topo-edge');
        const path = label.closest('svg')!.querySelector<SVGPathElement>(`.topo-track[data-topo-edge="${edge}"]`)!;
        const start = path.getPointAtLength(0), end = path.getPointAtLength(path.getTotalLength());
        const pill = label.querySelector<SVGPathElement>('.topo-edge-label-bg')!.getBBox();
        return Math.abs(pill.x + pill.width / 2 - (start.x + end.x) / 2);
      }));
    expect(lacpOffsets).toHaveLength(4);
    expect(lacpOffsets.every((offset) => offset < 1), JSON.stringify(lacpOffsets)).toBe(true);
    await panel.getByRole('button', { name: '1 column', exact: true }).click();
    const stackedHeight = await panel.locator('.topo-svg').evaluate((svg) => (svg as SVGSVGElement).viewBox.baseVal.height);
    expect(wideHeight).toBeLessThan(stackedHeight * .6);
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
    await panel.getByRole('button', { name: '1 column', exact: true }).click();

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
