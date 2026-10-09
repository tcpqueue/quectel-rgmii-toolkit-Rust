// Renders installer/assets/app.ico (the blue "5G" tile used by the device assistant page).
// Needs Playwright: PLAYWRIGHT_MODULE=/path/to/playwright node scripts/make-icon.cjs
const fs = require('node:fs');
const path = require('node:path');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');

const sizes = [16, 20, 24, 32, 40, 48, 64, 96, 256];
const tile = size => `<!doctype html><html><body style="margin:0;background:transparent">
<div style="width:${size}px;height:${size}px;display:grid;place-items:center;box-sizing:border-box;
border-radius:${size * 0.225}px;background:linear-gradient(160deg,#4aa3ff,#0a5cff);
color:#fff;font:700 ${size * 0.42}px/1 'Segoe UI','Helvetica Neue',Arial,sans-serif;letter-spacing:-.03em">5G</div>`;

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  const images = [];
  for (const size of sizes) {
    await page.setViewportSize({width: size, height: size});
    await page.setContent(tile(size));
    images.push(await page.screenshot({omitBackground: true, clip: {x: 0, y: 0, width: size, height: size}}));
  }
  await browser.close();
  // ICO with PNG entries (supported since Windows Vista).
  const header = Buffer.alloc(6 + 16 * images.length);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  let offset = header.length;
  images.forEach((png, i) => {
    const entry = 6 + 16 * i;
    header.writeUInt8(sizes[i] % 256, entry);
    header.writeUInt8(sizes[i] % 256, entry + 1);
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(png.length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += png.length;
  });
  const out = path.join(__dirname, '../installer/assets/app.ico');
  fs.writeFileSync(out, Buffer.concat([header, ...images]));
  console.log(`${out}: ${offset} bytes`);
})().catch(error => { console.error(error); process.exitCode = 1; });
