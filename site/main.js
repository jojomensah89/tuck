// Tuck website: live release facts, the island's accent, sponsor rails, the install stage and entrances.
(() => {
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* ---------- Latest release: version, size, file name and GitHub's own SHA-256 ---------- */
  const mb = bytes => `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  fetch('https://api.github.com/repos/jojomensah89/tuck/releases/latest', { headers: { Accept: 'application/vnd.github+json' } })
    .then(r => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
    .then(release => {
      const exe = (release.assets || []).find(a => /setup\.exe$/i.test(a.name) && /^https:\/\/github\.com\//.test(a.browser_download_url));
      if (!exe) return;
      const version = String(release.tag_name || '').replace(/^v/, '');
      // The hash always belongs to the file the button downloads: GitHub's digest of this asset, or nothing
      const digest = String(exe.digest || '').replace(/^sha256:/, '');
      const hash = /^[0-9a-f]{64}$/.test(digest) ? digest : 'Checksum unavailable for this release. Compare with SHA256SUMS.txt on the release page.';
      $$('[data-download]').forEach(a => { a.href = exe.browser_download_url; });
      $$('[data-size]').forEach(el => { el.textContent = mb(exe.size); });
      $$('[data-filename]').forEach(el => { el.textContent = exe.name; });
      $$('[data-release-line]').forEach(el => { el.textContent = `Version ${version}, ${mb(exe.size)}, for Windows 10 and 11 on x64.`; });
      $$('[data-hash]').forEach(el => { el.textContent = hash; });
    })
    .catch(() => { /* the build-time values stay, and they describe the same file as the build-time link */ });

  /* ---------- The island's artwork colour becomes the page's accent ---------- */
  const lum = c => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
  const contrastWithWhite = ({ r, g, b }) => 1.05 / (0.2126 * lum(r) + 0.7152 * lum(g) + 0.0722 * lum(b) + 0.05);
  const channel = v => typeof v === 'number' && Number.isFinite(v) && v >= 0 && v <= 255;
  const readout = document.querySelector('[data-readout]');
  const frame = document.querySelector('.island-frame');
  window.addEventListener('message', event => {
    // Only the island this page framed, from this site
    if (!frame || event.source !== frame.contentWindow || event.origin !== location.origin) return;
    if (event.data?.type !== 'tuck:accent') return;
    const raw = event.data.rgb;
    if (!raw || !channel(raw.r) || !channel(raw.g) || !channel(raw.b)) return;
    // Darken until white text on it passes AA, so the button stays readable whatever is playing
    let rgb = { r: raw.r, g: raw.g, b: raw.b };
    for (let i = 0; i < 20 && contrastWithWhite(rgb) < 4.6; i++) rgb = { r: rgb.r * 0.88, g: rgb.g * 0.88, b: rgb.b * 0.88 };
    document.documentElement.style.setProperty('--accent', `rgb(${Math.round(rgb.r)} ${Math.round(rgb.g)} ${Math.round(rgb.b)})`);
    if (readout) readout.textContent = 'This colour is the artwork playing in Tuck. Skip a track to change it.';
  });

  /* ---------- Fit the island on narrow screens without changing its insides ---------- */
  const fit = () => {
    if (!frame) return;
    const room = frame.parentElement.clientWidth;
    const scale = window.innerWidth <= 900 ? Math.min(1, room / 540) : 1;
    document.documentElement.style.setProperty('--island-scale', String(scale));
  };
  fit(); addEventListener('resize', fit);

  /* ---------- Sponsors: rails from sponsors.json, open spots counted ---------- */
  // Only http(s) links reach the page; anything else in sponsors.json is skipped, one entry at a time
  // Absolute http(s) addresses only: a missing, blank or relative value is no link at all
  const httpUrl = value => { if (typeof value !== 'string' || !value.trim()) return null; try { const u = new URL(value.trim()); return /^https?:$/.test(u.protocol) ? u : null; } catch { return null; } };
  const count = (value, fallback, max) => { const n = Number(value); return Number.isInteger(n) && n >= 0 && n <= max ? n : fallback; };
  const el = (tag, props = {}, text) => { const node = Object.assign(document.createElement(tag), props); if (text !== undefined) node.textContent = text; return node; };
  const MEGAPHONE = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m3 11 18-5v12L3 14v-3z"/><path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"/></svg>';
  fetch('sponsors.json', { cache: 'no-cache' }).then(r => r.json()).then(config => {
    const slots = count(config.slots, 8, 50);
    const price = count(config.priceUsd, 25, 100000);
    const buy = httpUrl(config.checkoutUrl) || httpUrl(config.contactUrl);
    const today = new Date().toISOString().slice(0, 10);
    const live = [];
    for (const s of Array.isArray(config.sponsors) ? config.sponsors : []) {
      if (live.length >= slots) break;
      if (!s || typeof s !== 'object') continue;
      const url = httpUrl(s.url);
      if (!url || typeof s.name !== 'string' || !s.name.trim()) continue;
      if (s.until && !(typeof s.until === 'string' && s.until >= today)) continue;
      // A logo can also be an image kept on this site (img/sponsors/...), as site/README.md describes
      const local = typeof s.logo === 'string' && /^img\/sponsors\/[\w.-]+$/.test(s.logo) ? new URL(s.logo, location.href) : null;
      const logo = httpUrl(s.logo) || local || httpUrl(`https://www.google.com/s2/favicons?domain=${url.hostname}&sz=128`);
      url.searchParams.set('ref', 'tuck');
      live.push({ name: s.name.trim(), tagline: typeof s.tagline === 'string' ? s.tagline : '', url: url.href, logo: logo?.href });
    }
    const open = slots - live.length;
    const taken = s => {
      const a = el('a', { className: 'slot slot--taken', href: s.url, target: '_blank', rel: 'sponsored noopener' });
      if (s.logo) {
        const img = el('img', { src: s.logo, alt: '', width: 44, height: 44, loading: 'lazy' });
        img.addEventListener('error', () => img.remove(), { once: true });
        a.append(img);
      }
      a.append(el('span', { className: 'slot__name' }, s.name));
      if (s.tagline) a.append(el('span', { className: 'slot__line' }, s.tagline));
      return a;
    };
    const vacant = () => {
      const a = el(buy ? 'a' : 'div', { className: 'slot slot--open' });
      if (buy) Object.assign(a, { href: buy.href, target: '_blank', rel: 'noopener' });
      a.setAttribute('aria-label', `Your startup here: $${price} a month, ${open} of ${slots} spots open`);
      a.insertAdjacentHTML('afterbegin', MEGAPHONE);
      a.append(el('span', { className: 'slot__label' }, 'Your startup here'), el('span', { className: 'slot__price', ariaHidden: 'true' }, `$${price} a month`));
      return a;
    };
    // Every slot is a card: sponsors in order, then the open ones. Wide screens split them over two rails.
    const cards = () => [...live.map(taken), ...Array.from({ length: open }, vacant)];
    const half = Math.ceil(slots / 2);
    const rails = { 0: () => cards().slice(0, half), 1: () => cards().slice(half) };
    $$('[data-rail]').forEach(rail => {
      const list = rails[rail.dataset.rail]?.() || [];
      const stack = el('div', { className: 'rail-stack' });
      stack.append(el('p', { className: 'rail-label' }, 'Sponsors'), ...list);
      rail.replaceChildren(...(list.length ? [stack] : []));
    });
    if (buy) $$('[data-sponsor-link]').forEach(a => { a.href = buy.href; a.target = '_blank'; a.rel = 'noopener'; });
    buildStrip(cards);
  }).catch(() => {});

  /* ---------- Sponsor strip: along the foot of the first screen on desktop, pinned to the bottom on phones ---------- */
  function buildStrip(cards) {
    const strip = document.querySelector('[data-strip]');
    if (!strip) return;
    const list = cards();
    if (!list.length) { strip.hidden = true; return; }
    const set = () => { const g = el('div', { className: 'strip__set' }); g.append(...cards()); return g; };
    const track = el('div', { className: 'strip__track' });
    const first = set();
    track.append(first);
    strip.replaceChildren(track);
    strip.hidden = false;
    document.documentElement.classList.add('has-strip');

    // A second, hidden copy makes the drift seamless; it is only added when the row is wider than the strip
    let copy = null;
    const fitCopy = () => {
      const needs = first.scrollWidth > strip.clientWidth;
      if (needs && !copy) {
        copy = set(); copy.setAttribute('aria-hidden', 'true'); copy.inert = true; track.append(copy);
      } else if (!needs && copy) { copy.remove(); copy = null; }
    };
    fitCopy(); addEventListener('resize', fitCopy);

    // Drift slowly; stop while pointed at, touched or focused, while off screen, and for reduced motion.
    // The strip is an ordinary sideways scroller, so a hand can always swipe it.
    let held = false, visible = true, resumeAt = 0, last = 0;
    const hold = () => { held = true; };
    const release = (delay = 0) => { held = false; resumeAt = performance.now() + delay; };
    strip.addEventListener('pointerenter', e => { if (e.pointerType === 'mouse') hold(); });
    strip.addEventListener('pointerleave', e => { if (e.pointerType === 'mouse') release(); });
    strip.addEventListener('pointerdown', hold);
    strip.addEventListener('pointerup', e => { if (e.pointerType !== 'mouse') release(2500); });
    strip.addEventListener('pointercancel', () => release(2500));
    strip.addEventListener('focusin', hold);
    strip.addEventListener('focusout', () => release(1500));
    strip.addEventListener('wheel', () => { held = true; clearTimeout(strip._wheel); strip._wheel = setTimeout(() => release(1500), 300); }, { passive: true });
    if ('IntersectionObserver' in window) new IntersectionObserver(([e]) => { visible = e.isIntersecting; }).observe(strip);
    const step = now => {
      const dt = last ? Math.min(64, now - last) : 0; last = now;
      if (copy && !reduced && !held && visible && !document.hidden && now >= resumeAt) {
        strip.scrollLeft += dt * 0.03; // about 30px a second
        const loop = copy.offsetLeft - first.offsetLeft;
        if (strip.scrollLeft >= loop) strip.scrollLeft -= loop;
      }
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);

    // On phones the strip is a bar at the bottom: it steps aside while reading down and returns on the way up,
    // when scrolling stops, and at the end of the page
    let lastY = scrollY, settle = 0;
    const phone = matchMedia('(max-width: 1279px)');
    addEventListener('scroll', () => {
      if (!phone.matches) return;
      const y = scrollY, atEnd = innerHeight + y >= document.documentElement.scrollHeight - 80;
      if (y > lastY + 4 && y > 200 && !atEnd) strip.classList.add('strip--away');
      else if (y < lastY - 4 || atEnd) strip.classList.remove('strip--away');
      lastY = y;
      clearTimeout(settle); settle = setTimeout(() => strip.classList.remove('strip--away'), 1200);
    }, { passive: true });
  }

  /* ---------- Install: the fragment walks through its three states; pointing at a statement holds it there ---------- */
  const stage = document.querySelector('.smartscreen');
  const steps = $$('.step');
  if (stage && steps.length) {
    let at = 0, held = false, timer = 0;
    const show = i => { at = i; stage.dataset.state = String(i); steps.forEach((s, n) => s.classList.toggle('on', n === i)); };
    const run = () => { clearInterval(timer); if (!reduced) timer = setInterval(() => { if (!held) show((at + 1) % steps.length); }, 3200); };
    steps.forEach((step, i) => {
      step.tabIndex = 0;
      const hold = () => { held = true; show(i); };
      const release = () => { held = false; };
      step.addEventListener('pointerenter', hold); step.addEventListener('focus', hold);
      step.addEventListener('pointerleave', release); step.addEventListener('blur', release);
    });
    show(0);
    // Only cycle while the section is on screen
    if ('IntersectionObserver' in window) new IntersectionObserver(([e]) => { if (e.isIntersecting) run(); else clearInterval(timer); }).observe(stage);
  }

  /* ---------- Entrances: each part of a section rises once, staggered ---------- */
  const reveals = $$('.reveal, .figures, .faq');
  reveals.forEach(el => [...(el.matches('.faq') ? el.querySelectorAll('.answers > p') : el.children)].forEach((child, i) => child.style.setProperty('--n', String(Math.min(i, 6)))));
  if (reduced || !('IntersectionObserver' in window)) reveals.forEach(el => el.classList.add('in'));
  else {
    const io = new IntersectionObserver(entries => entries.forEach(e => { if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); } }), { rootMargin: '0px 0px -12% 0px' });
    reveals.forEach(el => io.observe(el));
  }
})();
