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
      const context = await browser.newContext({ viewport: { width, height }, reducedMotion: 'reduce', permissions: ['clipboard-read', 'clipboard-write'] });
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
          const chart = page.locator('.benchmark-job');
          await chart.getByRole('button', { name: 'Runner cost', exact: true }).click();
          await chart.locator('.comparison', { hasText: '36 min' }).waitFor();
          assert.match(await chart.textContent(), /21 min/);
          assert.match(await chart.textContent(), /41.7%/);
          await chart.getByRole('button', { name: 'Runner time', exact: true }).click();
          await chart.locator('.comparison', { hasText: '29m 26s' }).waitFor();
          assert.match(await chart.textContent(), /16m 8s/);
          await page.locator('#how').getByRole('button', { name: 'Reuse' }).click();
          await page.locator('.flow-detail', { hasText: 'unread README' }).waitFor();

          const playground = page.locator('#try');
          const terminal = playground.locator('.term');
          async function runTest(expected) {
            await terminal.getByRole('button', { name: 'Run the tests', exact: true }).click();
            await terminal.locator('.hl').last().filter({ hasText: expected }).waitFor();
          }
          await runTest('EXECUTED');
          await runTest('CONFIRMED');
          await runTest('REPLAYED');
          await terminal.getByRole('button', { name: 'Edit the README', exact: true }).click();
          await runTest('REPLAYED');
          await terminal.getByRole('button', { name: 'Change the code', exact: true }).click();
          await runTest('EXECUTED');
          await terminal.locator('.term-out', { hasText: 'FAIL calc/calc.test.ts' }).waitFor();
          const prompt = terminal.getByRole('textbox', { name: 'Type a command' });
          await prompt.fill('cat calc/calc.ts');
          await prompt.press('Enter');
          await terminal.locator('.term-out', { hasText: 'return a - b;' }).waitFor();
          await prompt.press('ArrowUp');
          assert.equal(await prompt.inputValue(), 'cat calc/calc.ts');
          await terminal.getByRole('button', { name: 'Fix it', exact: true }).click();
          await runTest('REPLAYED');
          await terminal.getByRole('button', { name: 'What did it read?', exact: true }).click();
          await terminal.locator('.term-out', { hasText: 'newest of' }).waitFor();
          await playground.screenshot({ path: `${output}/${name}-terminal.png` });
          await terminal.getByRole('button', { name: 'Reset terminal', exact: true }).click();
          await runTest('EXECUTED');
          await terminal.getByRole('button', { name: 'Play example', exact: true }).click();
          await terminal.getByRole('button', { name: 'Stop demo', exact: true }).click();
          await terminal.getByRole('button', { name: 'Play example', exact: true }).waitFor();

          await playground.getByRole('button', { name: 'Change an input', exact: true }).click();
          await playground.getByRole('button', { name: 'Create missing config', exact: true }).click();
          await playground.locator('.mf-result', { hasText: 'run again' }).waitFor();
          assert.match(await playground.locator('.mf-table tr.bad').textContent(), /override.json/);
          await playground.getByRole('button', { name: 'Edit README.md', exact: true }).click();
          await playground.locator('.mf-result', { hasText: 'eligible for reuse' }).waitFor();
          await playground.screenshot({ path: `${output}/${name}-inputs.png` });

          await playground.getByRole('button', { name: 'Test an audit', exact: true }).click();
          const meter = playground.locator('.meter');
          assert(await meter.getByRole('button', { name: 'Audit: different result', exact: true }).isDisabled());
          await meter.getByRole('button', { name: 'Run next job', exact: true }).click();
          await meter.locator('.state', { hasText: 'recorded' }).waitFor();
          await meter.getByRole('button', { name: 'Run next job', exact: true }).click();
          await meter.locator('.state', { hasText: 'verified' }).waitFor();
          await meter.getByRole('button', { name: 'Audit: same result', exact: true }).click();
          await meter.locator('.meter-log', { hasText: 'per-test states agreed' }).waitFor();
          await meter.getByRole('button', { name: 'Audit: different result', exact: true }).click();
          await meter.locator('.state', { hasText: 'quarantined' }).waitFor();
          await meter.getByRole('button', { name: 'Run next job', exact: true }).click();
          assert.match(await meter.locator('.meter-log').textContent(), /stays quarantined/);
          await playground.screenshot({ path: `${output}/${name}-audit.png` });
          await meter.getByRole('button', { name: 'Reset verification', exact: true }).click();
          await meter.locator('.state', { hasText: 'new' }).waitFor();
          await playground.getByRole('button', { name: 'Try the terminal', exact: true }).click();

          const workflow = page.locator('.workflow-demo');
          await workflow.getByRole('button', { name: 'Before', exact: true }).click();
          assert.doesNotMatch(await workflow.locator('.diff').textContent(), /quod-action|id-token/);
          await workflow.getByRole('button', { name: 'With Quod', exact: true }).click();
          assert.match(await workflow.locator('.diff').textContent(), /quod-action@v2/);
          await workflow.getByRole('button', { name: 'Copy YAML', exact: true }).click();
          await workflow.getByRole('status').filter({ hasText: /^Copied$/ }).waitFor();
          const yaml = await page.evaluate(() => navigator.clipboard.readText());
          assert.match(yaml, /id-token: write/);
          assert.match(yaml, /- uses: pango07\/quod-action@v2/);
          assert.doesNotMatch(yaml, /^\+/m);
          assert(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), `${name}: interactive layout overflow`);

          await page.getByRole('link', { name: 'See the measured savings' }).click();
          assert.equal(new URL(page.url()).hash, '#benchmarks');
          for (const section of ['benchmarks', 'setup']) {
            await page.locator(`#${section}`).screenshot({ path: `${output}/${name}-${section}.png` });
          }
          const question = page.locator('summary', { hasText: 'Will my CI always be 40–50% cheaper?' });
          await question.click();
          assert(await question.evaluate(el => el.parentElement.open), 'FAQ must open');
          await page.locator('#faq').screenshot({ path: `${output}/${name}-faq.png` });
        } else if (route === '/under-the-hood') {
          await page.getByRole('button', { name: 'Handle disagreement' }).click();
          await page.locator('.seq-detail', { hasText: 'same promotion stamp' }).waitFor();
          await page.getByRole('button', { name: '36 min plain → 21 min with Quod', exact: true }).click();
          await page.locator('.receipt .explain', { hasText: '15 metered minutes' }).waitFor();
          await page.locator('.seq').screenshot({ path: `${output}/${name}-sequence.png` });
        }
      }
      assert.deepEqual(errors, [], `${name}: browser errors`);
      await context.close();
      console.log(`${name}: routes, layout, metric toggle, terminal edits/reuse/failure/reset, input changes, audit quarantine, workflow copy, stage navigation and FAQ passed`);
    }
    const motionContext = await browser.newContext({ viewport: { width: 1440, height: 1100 } });
    const motionPage = await motionContext.newPage();
    await motionPage.goto('http://localhost:3000', { waitUntil: 'networkidle' });
    await motionPage.locator('#how').scrollIntoViewIfNeeded();
    await motionPage.locator('#how').getByRole('button', { name: 'Pause stages', exact: true }).click();
    await motionPage.locator('#how').getByRole('button', { name: 'Reuse' }).click();
    assert.equal(await motionPage.locator('#how').getByRole('button', { name: 'Reuse' }).getAttribute('aria-pressed'), 'true');
    await motionPage.locator('#how').getByRole('button', { name: 'Play stages', exact: true }).click();
    await motionPage.waitForFunction(() => document.querySelector('#how .flow button[aria-pressed="true"]').textContent.includes('Record'), null, { timeout: 10000 });
    await motionContext.close();
    console.log('Standard motion: stage playback and pause passed');
  } finally {
    await browser.close();
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
