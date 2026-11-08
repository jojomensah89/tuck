import './refresh-feedback.css';

/** Complete whole 240ms turns, with at least 500ms feedback even for a fast reply. */
export function beginRefresh(button: HTMLButtonElement): (() => Promise<void>) | null {
  if (button.disabled || button.getAttribute('aria-busy') === 'true') return null;
  button.disabled = true;
  button.setAttribute('aria-busy', 'true');
  const start = performance.now();
  button.classList.add('refreshing');
  return async () => {
    const elapsed = performance.now() - start;
    await new Promise(resolve => setTimeout(resolve, Math.max(0, 500 - elapsed)));
    const animation = button.querySelector('svg')?.getAnimations().find(
      item => item instanceof CSSAnimation && item.animationName === 'refresh-turn',
    );
    if (animation?.effect) {
      // Use the animation's own clock so delayed paints cannot cut a turn short.
      const turns = Math.max(1, Math.ceil(Number(animation.currentTime ?? 0) / 240));
      animation.effect.updateTiming({ iterations: turns });
      // A motion-preference change or a removed button may cancel the animation.
      await animation.finished.catch(() => {});
    }
    button.classList.remove('refreshing');
    button.setAttribute('aria-busy', 'false');
    button.disabled = false;
  };
}
