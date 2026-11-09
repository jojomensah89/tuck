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
    if (!cards().length) { strip.hidden = true; return; }
    const set = () => { const g = el('div', { className: 'strip__set' }); g.append(...cards()); return g; };
    const track = el('div', { className: 'strip__track' });
    const first = set();
    track.append(first);
    strip.replaceChildren(track);
    strip.hidden = false;
    document.documentElement.classList.add('has-strip');

    // A second copy makes the drift seamless. Its cards stay clickable when they scroll into view, but screen
    // readers and the keyboard only meet the first set. It exists only while the row is wider than the strip.
    let copy = null;
    const fitCopy = () => {
      const style = getComputedStyle(strip);
      const room = strip.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
      const needs = first.scrollWidth > room;
      if (needs && !copy) {
        copy = set(); copy.setAttribute('aria-hidden', 'true');
        copy.querySelectorAll('a').forEach(a => { a.tabIndex = -1; });
        track.append(copy);
      } else if (!needs && copy) { copy.remove(); copy = null; strip.scrollLeft = 0; }
      kick();
    };

    // Drift slowly. Each reason to stand still is tracked on its own, and the drift resumes only when none is left.
    const motion = matchMedia('(prefers-reduced-motion: reduce)');
    const hold = { hover: false, touch: false, focus: false, wheel: false, offscreen: false };
    let touchTimer = 0, wheelTimer = 0, focusTimer = 0, frame = 0, last = 0;
    const moving = () => copy && !motion.matches && !document.hidden && !Object.values(hold).some(Boolean);
    const step = now => {
      frame = 0;
      if (!moving()) { last = 0; return; }
      const dt = last ? Math.min(64, now - last) : 0; last = now;
      strip.scrollLeft += dt * 0.03; // about 30px a second
      const loop = copy.offsetLeft - first.offsetLeft;
      if (strip.scrollLeft >= loop) strip.scrollLeft -= loop;
      frame = requestAnimationFrame(step);
    };
    // Frames are asked for only while something can move
    function kick() { if (!frame && moving()) { last = 0; frame = requestAnimationFrame(step); } }
    const set_ = (key, value) => { hold[key] = value; kick(); };
    strip.addEventListener('pointerenter', e => { if (e.pointerType === 'mouse') set_('hover', true); });
    strip.addEventListener('pointerleave', e => { if (e.pointerType === 'mouse') set_('hover', false); });
    strip.addEventListener('pointerdown', e => { if (e.pointerType !== 'mouse') { clearTimeout(touchTimer); set_('touch', true); } });
    const touchEnd = () => { clearTimeout(touchTimer); touchTimer = setTimeout(() => set_('touch', false), 2500); };
    strip.addEventListener('pointerup', e => { if (e.pointerType !== 'mouse') touchEnd(); });
    strip.addEventListener('pointercancel', touchEnd);
    strip.addEventListener('focusin', () => { clearTimeout(focusTimer); set_('focus', true); strip.classList.remove('strip--away'); });
    strip.addEventListener('focusout', () => { clearTimeout(focusTimer); focusTimer = setTimeout(() => set_('focus', strip.contains(document.activeElement)), 1500); });
    strip.addEventListener('wheel', () => { clearTimeout(wheelTimer); set_('wheel', true); wheelTimer = setTimeout(() => set_('wheel', false), 1500); }, { passive: true });
    if ('IntersectionObserver' in window) new IntersectionObserver(([e]) => set_('offscreen', !e.isIntersecting)).observe(strip);
    document.addEventListener('visibilitychange', kick);
    motion.addEventListener?.('change', kick);
    addEventListener('resize', fitCopy);
    fitCopy();

    // On phones the strip is a bar at the bottom: it steps aside while reading down and returns on the way up,
    // when scrolling stops, and at the end of the page. It never hides while one of its links has focus.
    const phone = matchMedia('(max-width: 1279px)');
    const maxY = () => Math.max(0, document.documentElement.scrollHeight - innerHeight);
    let anchorY = Math.min(scrollY, maxY()), settle = 0;
    const show = () => strip.classList.remove('strip--away');
    phone.addEventListener?.('change', () => { show(); anchorY = Math.min(scrollY, maxY()); });
    addEventListener('scroll', () => {
      if (!phone.matches) return;
      const y = Math.min(Math.max(scrollY, 0), maxY()); // ignores iOS overscroll bounce
      const atEnd = y >= maxY() - 80;
      // Direction counts once the page has moved 24px one way, so slow scrolling is still read
      if (atEnd || strip.contains(document.activeElement)) { show(); anchorY = y; }
      else if (y - anchorY > 24 && y > 200) { strip.classList.add('strip--away'); anchorY = y; }
      else if (anchorY - y > 24) { show(); anchorY = y; }
      else if ((y > anchorY && strip.classList.contains('strip--away')) || (y < anchorY && !strip.classList.contains('strip--away'))) anchorY = y;
      clearTimeout(settle); settle = setTimeout(show, 1200);
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
