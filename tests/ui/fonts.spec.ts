import { test, expect } from '@playwright/test';

test('bundled Latin and Chinese fonts render without external font requests', async ({ page }) => {
  const external: string[] = [];
  await page.route('**/*', route => {
    const url = new URL(route.request().url());
    if (url.protocol.startsWith('http') && url.hostname !== '127.0.0.1' && url.hostname !== 'localhost') {
      external.push(url.href);
      return route.abort();
    }
    return route.continue();
  });
  await page.goto('/');
  await expect(page.locator('.storage-total')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  const client = await page.context().newCDPSession(page);
  await client.send('DOM.enable');
  await client.send('CSS.enable');
  const { root } = await client.send('DOM.getDocument');
  const fonts = async (selector: string) => {
    const { nodeId } = await client.send('DOM.querySelector', { nodeId: root.nodeId, selector });
    return (await client.send('CSS.getPlatformFontsForNode', { nodeId })).fonts;
  };
  const latin = await fonts('.storage-total');
  const chinese = await fonts('h1');
  expect(latin.some(f => f.isCustomFont && /Geist/i.test(f.familyName) && f.glyphCount > 0)).toBe(true);
  expect(chinese.some(f => f.isCustomFont && /Noto Sans SC/i.test(f.familyName) && f.glyphCount > 0)).toBe(true);
  expect(external).toEqual([]);
  await page.screenshot({ path: 'artifacts/overview-fonts.png', fullPage: true });
});
