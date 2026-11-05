// Runs on the GitHub runner against the production build; never submits forms.
const assert = require('node:assert/strict');
const { mkdir } = require('node:fs/promises');
const { chromium } = require('playwright-core');

async function main() {
  const output = '/tmp/quod-landing-preview';
  await mkdir(output, { recursive: true });
  const browser = await chromium.launch({ executablePath: '/usr/bin/google-chrome', args: ['--no-sandbox'] });
  try {
    for (const [name, width, height] of [['desktop', 1440, 1100], ['mobile', 390, 1000]]) {
      const context = await browser.newContext({ viewport: { width, height }, reducedMotion: 'reduce' });
      const page = await context.newPage();
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      for (const [route, label] of [['/', 'home'], ['/under-the-hood', 'under-the-hood'], ['/docs', 'docs']]) {
        const response = await page.goto(`http://localhost:3000${route}`, { waitUntil: 'networkidle' });
        assert(response.ok(), `${route}: HTTP ${response.status()}`);
        await page.evaluate(() => document.fonts.ready);
        assert(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), `${name} ${route}: horizontal overflow`);
        await page.screenshot({ path: `${output}/${name}-${label}.png` });
        await page.screenshot({ path: `${output}/${name}-${label}-full.png`, fullPage: true });
        if (route === '/') {
          await page.getByRole('link', { name: 'See the measured savings' }).click();
          assert.equal(new URL(page.url()).hash, '#benchmarks');
          for (const section of ['benchmarks', 'setup']) {
            await page.locator(`#${section}`).screenshot({ path: `${output}/${name}-${section}.png` });
          }
          const question = page.locator('summary', { hasText: 'Will my CI always be 40–50% cheaper?' });
          await question.click();
          assert(await question.evaluate(el => el.parentElement.open), 'FAQ must open');
          await page.locator('#faq').screenshot({ path: `${output}/${name}-faq.png` });
        }
      }
      assert.deepEqual(errors, [], `${name}: browser errors`);
      await context.close();
      console.log(`${name}: routes, layout, benchmark navigation and FAQ passed`);
    }
  } finally {
    await browser.close();
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
