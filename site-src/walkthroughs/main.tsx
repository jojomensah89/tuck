import { createRoot } from 'react-dom/client'
import './kit/stageshow.css'
import './tuck.css'
import ShelfWalkthrough from './shelf'
import DictationWalkthrough from './dictation'

// Each [data-stage] panel on the page holds a still of the real island; the
// walkthrough replaces it once the script runs, and the still stays without it.
const scenes = { shelf: ShelfWalkthrough, dictation: DictationWalkthrough } as const

for (const panel of document.querySelectorAll<HTMLElement>('[data-stage]')) {
  const Scene = scenes[panel.dataset.stage as keyof typeof scenes]
  if (!Scene) continue
  const still = panel.querySelector('img')
  const mount = document.createElement('div')
  mount.className = 'walkthrough'
  mount.setAttribute('data-ui', '')
  mount.setAttribute('role', 'img')
  mount.setAttribute('aria-label', still?.alt ?? '')
  panel.append(mount)
  createRoot(mount).render(<Scene />)
  panel.classList.add('has-walkthrough')
}
