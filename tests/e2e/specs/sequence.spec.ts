import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test.describe('sequence tile', () => {
  test('renders participants, lifelines, messages, a self-message and a group frame', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="sequence"]');
    await expect(panel).toBeVisible();

    // Five participants (user, gateway, auth, db, orders, cache) → a lifeline each.
    // (user, gateway, auth, db, orders, cache = 6.)
    await expect(panel.locator('.seq-life')).toHaveCount(6);

    // Six messages → six hit areas.
    await expect(panel.locator('.seq-hit')).toHaveCount(6);

    // Different arrow styles are present: at least one dashed (reply) line.
    // (an SVG <line> whose y1 === y2 has a zero-height getBoundingClientRect in
    // Chromium, so Playwright's toBeVisible() reports it hidden even though it's
    // rendered — the same quirk timeline.spec.ts works around by asserting count
    // instead of visibility for <line> elements; boundingBox() confirms non-zero
    // width and a correct on-screen position.)
    await expect(panel.locator('.seq-line.reply')).toHaveCount(1);

    // The alt group frame + its else/expired compartment divider.
    await expect(panel.locator('.seq-frame').first()).toBeVisible();
    await expect(panel.locator('.seq-div')).toHaveCount(1);

    // Autonumber badges.
    await expect(panel.locator('.seq-num').first()).toBeVisible();

    // Full-width toggle offered.
    await expect(panel.locator('[data-widen]')).toHaveCount(1);
  });

  test('message hover shows a rich tooltip with core fields + a formatted link, escaping HTML', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="sequence"]');
    await expect(panel).toBeVisible();
    // Hover the first message's hit area.
    await panel.locator('.seq-hit').first().hover();
    const tip = page.locator('.wm-tip');
    await expect(tip).toBeVisible();
    await expect(tip).toContainText('user → gateway');
    await expect(tip).toContainText('type: sync');
    // The `trace` column has a --link format → a clickable link in the tooltip.
    await expect(tip.locator('a[href="https://trace.example.test/t-1"]')).toBeVisible();
    // The hostile `note` value is shown as text, never parsed as an element.
    await expect(tip.locator('img')).toHaveCount(0);
    await expect(tip).toContainText('onerror');
  });

  test('the mermaid button copies a valid sequenceDiagram to the clipboard', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="sequence"]');
    await expect(panel).toBeVisible();
    await panel.locator('[data-mermaid]').click();
    // The toast confirms the copy.
    await expect(page.locator('#toast')).toContainText('mermaid');
    const text = await page.evaluate(() => navigator.clipboard.readText());
    expect(text).toContain('sequenceDiagram');
    expect(text).toContain('autonumber');
    expect(text).toContain('->>');       // a sync arrow
    expect(text).toContain('-->>');      // a reply arrow
    expect(text).toContain('%% database'); // db participant annotation
    expect(text).toMatch(/\n\s*alt token valid/); // the group frame
    expect(text).toMatch(/\n\s*end/);
  });

  test('a loop group with a changing group-branch exports valid mermaid (no else/and)', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="sequence-loop"]');
    await expect(panel).toBeVisible();
    await panel.locator('[data-mermaid]').click();
    // The toast confirms the copy — the primary gate for this regression.
    await expect(page.locator('#toast')).toContainText('mermaid');
    const text = await page.evaluate(() => navigator.clipboard.readText());
    expect(text).toMatch(/\n\s*loop retry/);
    expect(text).toMatch(/\n\s*end/);
    // mermaid only allows `else` inside alt and `and` inside par — a loop frame
    // must never emit either compartment keyword, even though --group-branch
    // changes on every row here.
    expect(text).not.toMatch(/\n\s*else\b/);
    expect(text).not.toMatch(/\n\s*and\b/);
  });
});
