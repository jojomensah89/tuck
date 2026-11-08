import { SVG_ICONS } from './icons.js';
import { createRing } from './rings.js';
import { subscribeUpdate, updateAction } from './updater.js';
import './update-button.css';
export function mountUpdateButton(host: HTMLElement, settings = false): void {
  const button = document.createElement('button');
  button.type = 'button'; button.className = settings ? 'update-button update-sidebar' : 'rail-tab update-button';
  button.hidden = true;
  host.append(button);
  const ring = createRing(20, 1.5);
  const label = document.createElement('span');
  void subscribeUpdate(state => {
    button.hidden = !state.info;
    if (!settings) host.hidden = !state.info;
    if (!state.info) return;
    const name = state.phase === 'downloading' ? `Downloading update${state.progress === null ? '' : `, ${Math.round(state.progress)}%`}` : state.phase === 'restart' ? 'Restart to update' : state.phase === 'installing' ? 'Installing update' : `Update to v${state.info.version}`;
    button.setAttribute('aria-label', name); button.title = state.error || state.blocker || name;
    button.disabled = state.phase === 'downloading' || state.phase === 'installing';
    button.dataset.phase = state.phase; button.dataset.failed = String(!!state.error);
    const previousPhase = button.dataset.visualPhase;
    const oldIcon = previousPhase && previousPhase !== state.phase ? button.firstElementChild?.cloneNode(true) as HTMLElement | undefined : undefined;
    button.dataset.visualPhase = state.phase;
    button.innerHTML = state.phase === 'restart' ? SVG_ICONS.restart : SVG_ICONS.installUpdate;
    if (state.phase === 'downloading') { button.replaceChildren(ring.root); ring.set(state.progress); }
    if (oldIcon && button.checkVisibility() && !document.hidden && !document.querySelector('.keyboard-input')) {
      const icon = button.firstElementChild as HTMLElement;
      oldIcon.style.position = 'absolute'; button.append(oldIcon);
      oldIcon.animate([{opacity:1},{opacity:0}],{duration:100}).finished.then(()=>oldIcon.remove()).catch(()=>oldIcon.remove());
      icon.animate([{opacity:0},{opacity:1}],{duration:140});
    }
    if (settings) { label.textContent = name; button.append(label); }
  });
  button.addEventListener('click', e => { e.stopPropagation(); void updateAction().catch(error => console.error('[updater]', error)); });
}
