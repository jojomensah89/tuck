// Tuck website: live release facts, the island's accent, sponsor rails, the install stage and entrances.
(() => {
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* ---------- Latest release: version, size, file name and GitHub's own SHA-256 ---------- */
  const mb = bytes => `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  fetch('https://api.github.com/repos/jojomensah89/tuck/releases/latest', { headers: { Accept: 'application/vnd.github+json' } })
    .then(r => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
    .then(release => {
      const exe = (release.assets || []).find(a => /setup\.exe$/i.test(a.name));
      if (!exe) return;
      const version = String(release.tag_name || '').replace(/^v/, '');
      $$('[data-download]').forEach(a => { a.href = exe.browser_download_url; });
      $$('[data-size]').forEach(el => { el.textContent = mb(exe.size); });
      $$('[data-filename]').forEach(el => { el.textContent = exe.name; });
      $$('[data-release-line]').forEach(el => { el.textContent = `Version ${version}, ${mb(exe.size)}, for Windows 10 and 11 on x64.`; });
      const digest = String(exe.digest || '').replace(/^sha256:/, '');
      if (/^[0-9a-f]{64}$/.test(digest)) $$('[data-hash]').forEach(el => { el.textContent = digest; });
    })
    .catch(() => { /* the build-time values stay */ });

  /* ---------- The island's artwork colour becomes the page's accent ---------- */
  const lum = c => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
  const contrastWithWhite = ({ r, g, b }) => 1.05 / (0.2126 * lum(r) + 0.7152 * lum(g) + 0.0722 * lum(b) + 0.05);
  const readout = document.querySelector('[data-readout]');
  const frame = document.querySelector('.island-frame');
  window.addEventListener('message', event => {
    if (!frame || event.source !== frame.contentWindow || event.data?.type !== 'tuck:accent') return;
    let rgb = event.data.rgb;
    if (!rgb) return;
    // Darken until white text on it passes AA, so the button stays readable whatever is playing
    rgb = { r: rgb.r, g: rgb.g, b: rgb.b };
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
  const esc = s => String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const withRef = url => { try { const u = new URL(url); u.searchParams.set('ref', 'tuck'); return u.href; } catch { return url; } };
  fetch('sponsors.json', { cache: 'no-cache' }).then(r => r.json()).then(config => {
    const today = new Date().toISOString().slice(0, 10);
    const live = (config.sponsors || []).filter(s => s.name && s.url && (!s.until || s.until >= today)).slice(0, config.slots);
    const open = Math.max(0, config.slots - live.length);
    const buy = config.checkoutUrl || config.contactUrl;
    const card = s => `<a class="sponsor" href="${esc(withRef(s.url))}" rel="sponsored noopener" target="_blank">
      <img src="${esc(s.logo || `https://www.google.com/s2/favicons?domain=${new URL(s.url).hostname}&sz=64`)}" alt="" width="32" height="32" loading="lazy">
      <strong>${esc(s.name)}</strong><span>${esc(s.tagline || '')}</span></a>`;
    const openCard = `<a class="sponsor open" href="${esc(buy)}" target="_blank" rel="noopener">
      <strong>Your product here</strong><span>$${config.priceUsd} a month beside every Tuck download. ${open} of ${config.slots} spots open.</span></a>`;
    // Sponsors alternate left and right; the open spot goes on the shorter rail
    const left = live.filter((_, i) => i % 2 === 0), right = live.filter((_, i) => i % 2 === 1);
    const openOn = left.length <= right.length ? '0' : '1';
    // The other rail explains what a sponsor gets, so neither side of the column stands empty
    const aboutCard = `<a class="sponsor open" href="${esc(buy)}" target="_blank" rel="noopener">
      <strong>Sponsor Tuck</strong><span>Your logo, name and one line, linked, on every screen of this page. Monthly, cancel any time.</span></a>`;
    const rails = {
      0: left.map(card).join('') + (open && openOn === '0' ? openCard : '') + (open && openOn === '1' ? aboutCard : ''),
      1: right.map(card).join('') + (open && openOn === '1' ? openCard : '') + (open && openOn === '0' ? aboutCard : ''),
      inline: live.map(card).join('') + (open ? openCard + aboutCard : ''),
    };
    $$('[data-rail]').forEach(rail => {
      const id = rail.dataset.rail;
      if (!rails[id]) { rail.innerHTML = ''; return; }
      rail.innerHTML = `<div class="rail-stack">${rails[id]}</div>`;
    });
    $$('[data-sponsor-link]').forEach(a => { a.href = buy; a.target = '_blank'; a.rel = 'noopener'; });
  }).catch(() => {});

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
