import { expect, type Locator } from '@playwright/test';

// Wait for the real render and responsive resize, rather than assuming a fixed
// delay is long enough on every runner. Keep application animations enabled.
export async function waitForChart(canvas: Locator): Promise<void> {
  await expect(canvas).toBeVisible();
  await canvas.evaluate(async (el) => {
    await document.fonts.ready;
    await Promise.all((el.closest('.panel')?.getAnimations() || [])
      .filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
      .map((animation) => animation.finished));
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
  });
  await expect.poll(() => canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el);
    return !!chart && chart.width > 0 && chart.height > 0
      && chart.data.datasets.every((_: unknown, index: number) =>
        chart.getDatasetMeta(index).data.every((point: any) =>
          Object.values(point.$animations || {}).every((animation: any) => !animation.active())));
  })).toBe(true);
}
