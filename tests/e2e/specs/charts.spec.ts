import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test('chart canvases recover after browser zoom changes', async ({ page, context }) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto(`/session/${SESSION_ID}/`);
  const canvas = page.locator('.panel[data-tile="by-day"] canvas');
  await canvas.scrollIntoViewIfNeeded();
  const measure = () => canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el), rect = el.getBoundingClientRect();
    const parent = el.parentElement!.getBoundingClientRect();
    const pixels = chart.ctx.getImageData(0, 0, el.width, el.height).data;
    let bitmapHash = 2166136261;
    for (let i = 0; i < pixels.length; i += 17) bitmapHash = Math.imul(bitmapHash ^ pixels[i], 16777619);
    return { innerWidth, dpr: devicePixelRatio, attrWidth: el.width, attrHeight: el.height,
      clientWidth: el.clientWidth, clientHeight: el.clientHeight, rectWidth: rect.width,
      rectHeight: rect.height, parentWidth: parent.width, chartWidth: chart.width,
      chartHeight: chart.height, styleWidth: el.style.width, styleHeight: el.style.height,
      bitmapHash, chartDpr: chart.currentDevicePixelRatio,
      points: chart.getDatasetMeta(0).data.map((p: any) => [p.x, p.y]),
    };
  });
  await page.waitForTimeout(1200);
  const before = await measure();
  // The canvas backing store covers every physical pixel after both browser
  // DPR and muckdb's own CSS interface scale; otherwise charts look enlarged
  // and blurry even though their geometry is correct.
  expect(before.chartDpr).toBeGreaterThanOrEqual(2);
  expect(before.attrWidth).toBeCloseTo(before.rectWidth * before.chartDpr / 1.25, 0);
  expect(before.attrHeight).toBeCloseTo(before.rectHeight * before.chartDpr / 1.25, 0);
  const cdp = await context.newCDPSession(page);
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 1024, height: 576, deviceScaleFactor: 1.25, mobile: false,
    screenWidth: 1280, screenHeight: 720,
  });
  await page.waitForTimeout(20);
  const zoomed = await measure();
  await cdp.send('Emulation.clearDeviceMetricsOverride');
  await page.waitForTimeout(500);
  const restored = await measure();
  expect(zoomed.dpr).toBe(1.25);
  expect(restored.chartWidth).toBe(before.chartWidth);
  expect(restored.chartHeight).toBe(before.chartHeight);
  expect(restored.chartDpr).toBe(before.chartDpr);
  expect(restored.attrWidth).toBeCloseTo(restored.rectWidth * restored.chartDpr / 1.25, 0);
  expect(restored.attrHeight).toBeCloseTo(restored.rectHeight * restored.chartDpr / 1.25, 0);
  expect(restored.rectWidth).toBeCloseTo(before.rectWidth, 0);
  expect(restored.bitmapHash).toBe(before.bitmapHash);
  expect(restored.points).toEqual(before.points);
});

for (const kind of ['scatter', 'pie']) {
  test(`${kind} hover selects every point under console zoom`, async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const canvas = page.locator('.panel[data-tile="by-day"] canvas');
    await canvas.scrollIntoViewIfNeeded();
    await canvas.evaluate((el, kind) => {
      const Chart = (window as any).Chart;
      Chart.getChart(el).destroy();
      new Chart(el, {
        type: kind,
        data: { labels: ['a', 'b', 'c', 'd', 'e'], datasets: [{
          data: kind === 'scatter' ? [1, 2, 3, 4, 5].map(x => ({ x, y: 3 })) : [1, 1, 1, 1, 1],
          pointRadius: 8,
        }] },
        options: { animation: false, responsive: true, maintainAspectRatio: false,
          scales: kind === 'scatter' ? { x: { min: 0, max: 6 }, y: { min: 0, max: 6 } } : {},
          plugins: { legend: { display: false } } },
      });
    }, kind);
    await page.waitForTimeout(500);
    for (const index of [0, 1, 2, 3, 4, 2]) {
      const p = await canvas.evaluate((el, index) => {
        const c = (window as any).Chart.getChart(el);
        const p = c.getDatasetMeta(0).data[index].getCenterPoint(true), r = el.getBoundingClientRect();
        return { x: r.left + p.x * r.width / c.width, y: r.top + p.y * r.height / c.height };
      }, index);
      await page.mouse.move(p.x, p.y);
      await expect.poll(() => canvas.evaluate(el => (window as any).Chart.getChart(el).tooltip.getActiveElements()[0]?.index)).toBe(index);
    }
  });
}

for (const [name, type] of [['line', 'line'], ['area', 'line'], ['scatter', 'scatter'], ['bar', 'bar']]) {
  test(`${name} data hover does not show the x-axis tooltip`, async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const canvas = page.locator('.panel[data-tile="by-day"] canvas');
    await canvas.scrollIntoViewIfNeeded();
    await canvas.evaluate((el, { name, type }) => {
      const Chart = (window as any).Chart;
      Chart.getChart(el).destroy();
      const scatter = type === 'scatter';
      new Chart(el, {
        type,
        data: scatter
          ? { datasets: [{ data: [1, 2, 3, 4, 5].map(x => ({ x, y: x })), pointRadius: 8 }] }
          : { labels: ['a', 'b', 'c', 'd', 'e'], datasets: [{ data: [1, 2, 3, 4, 5], fill: name === 'area' }] },
        options: {
          animation: false, responsive: true, maintainAspectRatio: false,
          plugins: { legend: { display: false }, muckTimeTickTooltip: { enabled: true, tz: 'utc' } },
        },
      });
    }, { name, type });
    const point = await canvas.evaluate((el) => {
      const chart = (window as any).Chart.getChart(el);
      const p = chart.getDatasetMeta(0).data[2].getCenterPoint(true);
      const rect = el.getBoundingClientRect();
      return { x: rect.left + p.x * rect.width / chart.width, y: rect.top + p.y * rect.height / chart.height };
    });
    await page.mouse.move(point.x, point.y);
    await page.waitForTimeout(100);
    await expect(page.locator('.chart-time-tip')).not.toBeVisible();
  });
}

test('chart tiles render canvases; table tile renders a table', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);

  // Bar and line tiles draw with Chart.js → a <canvas> inside their panel.
  await expect(page.locator('.panel', { hasText: 'By category' }).locator('canvas')).toBeVisible();
  await expect(page.locator('.panel', { hasText: 'By day' }).locator('canvas')).toBeVisible();

  // The table tile renders an HTML table (miniTable), not a canvas.
  await expect(page.locator('.panel', { hasText: 'All widgets' }).locator('table')).toBeVisible();
});

test('bar tooltips follow the final three hovered bars under console zoom', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  const panel = page.locator('.panel', { hasText: 'By category' });
  const canvas = panel.locator('canvas');
  const hits = panel.locator('.muck-bar-hit');
  await expect(hits).toHaveCount(5);
  for (const requestedIndex of [-3, -2, -1]) {
    const index = await canvas.evaluate((el, requestedIndex) => {
      const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
      const bars = chart.getDatasetMeta(0).data;
      return requestedIndex < 0 ? bars.length + requestedIndex : requestedIndex;
    }, requestedIndex);
    await hits.nth(index).dispatchEvent('pointerenter');
    await expect.poll(() => canvas.evaluate((el) => (window as any).Chart.getChart(el as HTMLCanvasElement).tooltip.getActiveElements()[0]?.index)).toBe(index);
  }
});

test('line snapping follows late points under console zoom', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  const canvas = page.locator('.panel[data-tile="by-day"] canvas');
  await canvas.scrollIntoViewIfNeeded();
  await page.evaluate(() => window.scrollBy(0, 250));
  await page.waitForTimeout(1000);
  const point = await canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
    const points = chart.getDatasetMeta(0).data;
    const index = points.length - 2;
    const p = points[index].getCenterPoint();
    const rect = el.getBoundingClientRect();
    return {
      index,
      x: rect.left + p.x * rect.width / chart.width,
      y: rect.top + p.y * rect.height / chart.height,
    };
  });
  const picked = await canvas.evaluate((el, point) => {
    const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
    return (window as any).Chart.Interaction.modes.muckLineSnap(
      chart,
      { native: { clientX: point.x, clientY: point.y }, x: 0, y: 0 },
      { snapDistance: 14 },
      true,
    )[0]?.index;
  }, point);
  expect(picked).toBe(point.index);
});

test('multi-series line snapping follows late x positions under console zoom', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  const canvas = page.locator('.panel[data-tile="by-day-multi"] canvas');
  await canvas.scrollIntoViewIfNeeded();
  await page.evaluate(() => window.scrollBy(0, 250));
  await page.waitForTimeout(1000);
  for (const index of [30, 36, 40, 45, 48, 49, 40]) {
    const target = await canvas.evaluate((el, index) => {
      const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
      const p = chart.getDatasetMeta(0).data[index].getCenterPoint(true);
      const rect = el.getBoundingClientRect();
      return { x: rect.left + p.x * rect.width / chart.width, y: rect.top + p.y * rect.height / chart.height };
    }, index);
    await page.mouse.move(target.x, target.y);
    await expect.poll(() => canvas.evaluate((el) =>
      (window as any).Chart.getChart(el as HTMLCanvasElement).tooltip.getActiveElements().map((a: any) => a.index)
    )).toEqual([index, index]);
    await expect.poll(() => canvas.evaluate((el) => {
      const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
      return Number.isInteger(chart.$muckHoverTickIndex)
        && chart.$muckHoverTickPainted === chart.$muckHoverTickIndex;
    })).toBe(true);
    const highlighted = await canvas.evaluate((el) => {
      const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
      const tick = chart.$muckHoverTickIndex, context = { chart, index: tick };
      const ticks = chart.config.options.scales.x.ticks, grid = chart.config.options.scales.x.grid;
      return {
        tick,
        painted: chart.$muckHoverTickPainted,
        label: ticks.color(context),
        mark: grid.tickColor(context),
        weight: ticks.font(context).weight,
        primary: getComputedStyle(document.documentElement).getPropertyValue('--primary').trim(),
      };
    });
    expect(highlighted.tick).toBeGreaterThanOrEqual(0);
    expect(highlighted.painted).toBe(highlighted.tick);
    expect(highlighted.label).toBe(highlighted.primary);
    expect(highlighted.mark).toBe(highlighted.primary);
    expect(highlighted.weight).toBe('bold');
  }
});

test('multi-series line hover ignores a series after its final point', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  const canvas = page.locator('.panel[data-tile="by-day-multi"] canvas');
  await canvas.scrollIntoViewIfNeeded();
  await canvas.evaluate((el) => {
    const Chart = (window as any).Chart;
    Chart.getChart(el as HTMLCanvasElement).destroy();
    new Chart(el, {
      type: 'line',
      data: { datasets: [
        { label: 'ends at 29', data: Array.from({ length: 50 }, (_, i) => ({ x: i + 1, y: i < 29 ? i : null })) },
        { label: 'continues', data: Array.from({ length: 50 }, (_, i) => ({ x: i + 1, y: i })) },
      ] },
      options: { animation: false, responsive: true, maintainAspectRatio: false,
        interaction: { mode: 'muckLineIndex', axis: 'x', intersect: false },
        plugins: { muckHoverCursor: { enabled: true } },
        scales: { x: { type: 'linear' } },
      },
    });
  });
  for (const index of [28, 29, 35, 45, 49]) {
    const target = await canvas.evaluate((el, index) => {
      const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
      const p = chart.getDatasetMeta(1).data[index].getCenterPoint(true);
      const rect = el.getBoundingClientRect();
      return { x: rect.left + p.x * rect.width / chart.width, y: rect.top + p.y * rect.height / chart.height };
    }, index);
    await page.mouse.move(target.x, target.y);
    await expect.poll(() => canvas.evaluate((el) => {
      const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
      return chart.tooltip.getActiveElements().map((a: any) => ({ dataset: a.datasetIndex, index: a.index }));
    })).toEqual(index < 29
      ? [{ dataset: 0, index }, { dataset: 1, index }]
      : [{ dataset: 1, index }]);
  }
});

test('Cartesian charts brush-zoom their x-range and can reset', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  const panel = page.locator('.panel[data-tile="by-day"]');
  const canvas = panel.locator('canvas');
  const box = await canvas.boundingBox();
  if (!box) throw new Error('chart canvas has no bounds');

  await canvas.evaluate((el) => {
    const r = el.getBoundingClientRect();
    const y = r.top + r.height / 2;
    const fire = (type: string, x: number) => el.dispatchEvent(new MouseEvent(type, {
      bubbles: true, button: 0, clientX: x, clientY: y,
    }));
    fire('mousedown', r.left + r.width * 0.2);
    fire('mousemove', r.left + r.width * 0.65);
    fire('mouseup', r.left + r.width * 0.65);
  });

  const reset = panel.locator('.chart-reset');
  await expect(reset).toBeVisible();
  const zoomed = await canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
    return { min: chart.options.scales.x.min, max: chart.options.scales.x.max };
  });
  expect(zoomed.min).not.toBeUndefined();
  expect(zoomed.max).not.toBeUndefined();

  await reset.click();
  await expect(reset).toBeHidden();
});

test('legend clicks fade a dataset rather than brushing or hiding it', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  const canvas = page.locator('.panel[data-tile="by-day"] canvas');
  const faded = await canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
    chart.options.plugins.legend.display = true;
    chart.update();
    const hit = chart.legend.legendHitBoxes[0];
    const r = el.getBoundingClientRect();
    const x = r.left + (hit.left + hit.width / 2) * r.width / chart.width;
    const y = r.top + (hit.top + hit.height / 2) * r.height / chart.height;
    const fire = (type: string) => el.dispatchEvent(new MouseEvent(type, {
      bubbles: true, button: 0, clientX: x, clientY: y,
    }));
    fire('mousedown'); fire('mouseup'); fire('click');
    return {
      faded: chart.$muckFadedDatasets[0],
      hidden: chart.getDatasetMeta(0).hidden,
      resetVisible: !chart.$muckBrushZoom.reset.hidden,
    };
  });
  expect(faded.faded).toBe(true);
  expect(faded.hidden).not.toBe(true);
  expect(faded.resetVisible).toBe(false);

  const restored = await canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
    const hit = chart.legend.legendHitBoxes[0], r = el.getBoundingClientRect();
    const x = r.left + (hit.left + hit.width / 2) * r.width / chart.width;
    const y = r.top + (hit.top + hit.height / 2) * r.height / chart.height;
    el.dispatchEvent(new MouseEvent('click', { bubbles: true, button: 0, clientX: x, clientY: y }));
    return !!chart.$muckFadedDatasets[0];
  });
  expect(restored).toBe(false);

});
