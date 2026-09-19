import { chromium } from 'playwright-core';

const OUT = '/tmp/zeron-splitdemo-test/thread-tools';
const log: string[] = [];
const browser = await chromium.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const page = await browser.newPage({ viewport: { width: 1140, height: 960 } });
page.on('pageerror', e => log.push('PAGEERROR ' + e.message));
page.on('console', m => { if (m.type() === 'error') log.push('CONSOLE ' + m.text()); });
await page.goto('file:///Users/reinamaccredy/Code/zeron-fork/docs/research/thread-tools/demo.html');

const shot = async (n: string) => { await page.screenshot({ path: `${OUT}/${n}.png` }); };
const order = async () => page.evaluate(() => [...document.querySelectorAll('#lists .row')].map(r => {
  const el = r as HTMLElement;
  return (el.closest('.nest') ? '↳' : '') + el.dataset.id + (el.closest('.archived') ? '(arch)' : '');
}));

await shot('01-default');
log.push('default order=' + JSON.stringify(await order()));

await page.check('input[name="rank"][value="max"]');
await shot('02-rank-max');
log.push('rank=max order=' + JSON.stringify(await order()));
await page.check('input[name="rank"][value="parent"]');

await page.check('input[name="badge"][value="by"]');
await shot('03-badge-by');
log.push('badges=' + JSON.stringify(await page.evaluate(() => [...document.querySelectorAll('.by')].map(b => b.textContent))));
await page.check('input[name="badge"][value="none"]');

await page.check('#archived');
await shot('04-parent-archived');
log.push('archived order=' + JSON.stringify(await order()));
await page.uncheck('#archived');

await page.check('input[name="held"][value="now"]');
log.push('held badges after now=' + await page.evaluate(() => document.querySelectorAll('.xmsg.held').length));
await page.check('input[name="held"][value="held"]');
log.push('held badges after held=' + await page.evaluate(() => document.querySelectorAll('.xmsg.held').length));

await page.click('.tool .chip[data-open="c1"]');
await shot('05-open-child-from-chip');
log.push('selected after chip=' + JSON.stringify(await page.evaluate(() => [...document.querySelectorAll('.row.selected')].map(r => (r as HTMLElement).dataset.id))) + ' title=' + await page.evaluate(() => document.querySelector('.titlebar .center')!.firstChild!.textContent!.trim()));

await browser.close();
console.log(log.join('\n'));
