import { chromium } from 'playwright-core';

const OUT = '/tmp/zeron-splitdemo-test/shots';
const log: string[] = [];
const browser = await chromium.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const page = await browser.newPage({ viewport: { width: 1240, height: 860 } });
page.on('pageerror', e => log.push('PAGEERROR ' + e.message));
page.on('console', m => { if (m.type() === 'error') log.push('CONSOLE ' + m.text()); });
await page.goto('file:///Users/reinamaccredy/Code/zeron-fork/docs/research/split-view/demo.html');

const shot = async (n: string) => { await page.screenshot({ path: `${OUT}/${n}.png` }); };
const center = async (sel: string) => { const b = (await page.locator(sel).first().boundingBox())!; return { x: b.x + b.width / 2, y: b.y + b.height / 2 }; };
const panes = async () => page.evaluate(() => [...document.querySelectorAll('.pane')].map(p => (p as HTMLElement).dataset.id + (p.classList.contains('focus') ? '*' : '')));
const dragTo = async (fromSel: string, to: { x: number; y: number }, drop = true, mid?: (() => Promise<void>)) => {
  const f = await center(fromSel);
  await page.mouse.move(f.x, f.y); await page.mouse.down();
  await page.mouse.move(f.x + 12, f.y + 12, { steps: 3 });
  await page.mouse.move(to.x, to.y, { steps: 12 });
  if (mid) await mid();
  if (drop) await page.mouse.up();
};

await shot('01-idle');
log.push('idle panes=' + JSON.stringify(await panes()));

// 2: drag tab s2 over main -> zones appear, hover right zone
const mainBox = (await page.locator('#main').boundingBox())!;
const rightHalf = { x: mainBox.x + mainBox.width * 0.85, y: mainBox.y + mainBox.height / 2 };
const leftHalf = { x: mainBox.x + mainBox.width * 0.15, y: mainBox.y + mainBox.height / 2 };
await dragTo('.tab[data-id="s2"]', rightHalf, true, async () => {
  await shot('02-tab-drag-hover-right');
  log.push('hover-right zones=' + JSON.stringify(await page.evaluate(() => [...document.querySelectorAll('#dz [data-zone]')].map(z => z.getAttribute('data-zone') + ':' + z.classList.contains('hot')))));
  log.push('dragging class=' + await page.evaluate(() => document.querySelector('#main')!.classList.contains('dragging')));
  log.push('ghost visible=' + await page.evaluate(() => !(document.querySelector('#ghost') as HTMLElement).hidden));
});
await shot('03-after-drop-right');
log.push('after drop-right panes=' + JSON.stringify(await panes()) + ' split=' + await page.evaluate(() => document.querySelector('#main')!.classList.contains('split')));
log.push('tabs active=' + JSON.stringify(await page.evaluate(() => [...document.querySelectorAll('.tab.active')].map(t => (t as HTMLElement).dataset.id))));
log.push('rows selected=' + JSON.stringify(await page.evaluate(() => [...document.querySelectorAll('.row.selected')].map(t => (t as HTMLElement).dataset.id))));
log.push('rpane follows=' + await page.evaluate(() => document.querySelector('#rpane .follow')!.textContent));

// 5: drag sidebar row s3 while split -> replace pane 0
await dragTo('.row[data-id="s3"]', leftHalf, true, async () => {
  await shot('04-row-drag-replace-hover');
  log.push('replace zones=' + JSON.stringify(await page.evaluate(() => [...document.querySelectorAll('#dz [data-zone]')].map(z => z.getAttribute('data-zone') + ':' + z.classList.contains('hot') + ':' + z.textContent!.trim()))));
});
await shot('05-after-replace');
log.push('after replace panes=' + JSON.stringify(await panes()));

// swap: drag s2 (right pane) onto left pane
await dragTo('.row[data-id="s2"]', leftHalf);
log.push('after swap panes=' + JSON.stringify(await panes()));

// click pane 1 to focus
const p1 = await center('.pane[data-index="1"] .transcript');
await page.mouse.click(p1.x, p1.y);
log.push('after focus click panes=' + JSON.stringify(await panes()) + ' rpane=' + await page.evaluate(() => document.querySelector('#rpane .follow')!.textContent));

// 6: divider drag + snap + dblclick
const div = await center('.divider');
await page.mouse.move(div.x, div.y); await page.mouse.down(); await page.mouse.move(div.x - 160, div.y, { steps: 8 }); await page.mouse.up();
log.push('after divider drag ratio=' + await page.evaluate(() => getComputedStyle(document.querySelector('#main')!).getPropertyValue('--w')));
await shot('06-divider-dragged');
const div2 = await center('.divider');
await page.mouse.move(div2.x, div2.y); await page.mouse.down(); await page.mouse.move(div2.x - 700, div2.y, { steps: 8 }); await page.mouse.up();
log.push('after divider min-clamp ratio=' + await page.evaluate(() => getComputedStyle(document.querySelector('#main')!).getPropertyValue('--w')) + ' pane0 width=' + (await page.locator('.pane').first().boundingBox())!.width);
await page.locator('.divider').dblclick();
log.push('after dblclick ratio=' + await page.evaluate(() => getComputedStyle(document.querySelector('#main')!).getPropertyValue('--w')));

// Esc cancels
await dragTo('.tab[data-id="s1"]', rightHalf, false, async () => { await page.keyboard.press('Escape'); });
await page.mouse.up();
log.push('after esc panes=' + JSON.stringify(await panes()) + ' dragging=' + await page.evaluate(() => document.querySelector('#main')!.classList.contains('dragging')) + ' ghost hidden=' + await page.evaluate(() => (document.querySelector('#ghost') as HTMLElement).hidden));

// drop outside main (onto sidebar) cancels
const sb = await center('#sidebar .group-header');
await dragTo('.tab[data-id="s1"]', sb);
log.push('after drop-outside panes=' + JSON.stringify(await panes()));

// 7: close pane 0
await page.locator('.pane[data-index="0"] .x').click();
await shot('07-after-close');
log.push('after close panes=' + JSON.stringify(await panes()) + ' split=' + await page.evaluate(() => document.querySelector('#main')!.classList.contains('split')));

// same-session no-op when single: drag the displayed session
const cur = (await panes())[0].replace('*', '');
await dragTo(`.tab[data-id="${cur}"]`, leftHalf, true, async () => {
  await shot('08-same-session-zones');
  log.push('same-session zones=' + JSON.stringify(await page.evaluate(() => [...document.querySelectorAll('#dz [data-zone]')].map(z => z.getAttribute('data-zone') + ':' + z.classList.contains('disabled') + ':' + z.classList.contains('hot')))));
});
log.push('after same-session drop panes=' + JSON.stringify(await panes()));

// plain click on a row selects (no drag)
await page.locator('.row[data-id="s4"]').click();
log.push('after plain click panes=' + JSON.stringify(await panes()));

// split left via sidebar row
await dragTo('.row[data-id="s5"]', leftHalf);
await shot('09-split-left-from-row');
log.push('after split-left panes=' + JSON.stringify(await panes()));

// chip ghost mode
await page.locator('#ghostmode button[data-g="chip"]').click();
await dragTo('.tab[data-id="s1"]', rightHalf, false, async () => { await shot('10-chip-ghost'); });
await page.mouse.up();

console.log(log.join('\n'));
await browser.close();
