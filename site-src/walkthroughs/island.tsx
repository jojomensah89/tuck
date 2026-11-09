import type { CSSProperties, ReactNode } from 'react'

/**
 * Tuck's island as drawn for the walkthroughs: one black shape hanging from the
 * top edge of the screen, its content a separate layer so nothing stretches.
 * Sizes follow the app (idle 212 × 32, open about 440 wide), scaled to the stage.
 */
export type IslandBox = { w: number; h: number; r: number }

export function Island({ box, cx, children, label, className }: { box: IslandBox; cx: number; children?: ReactNode; label?: string; className?: string }) {
  const style: CSSProperties = { left: cx - box.w / 2, width: box.w, height: box.h, borderBottomLeftRadius: box.r, borderBottomRightRadius: box.r }
  return (
    <div className={`tk-island ${className ?? ''}`} style={style} data-target="island" aria-label={label}>
      <Ear side="l" />
      <Ear side="r" />
      <div className="tk-island__body">{children}</div>
    </div>
  )
}

/** The desktop behind: the same calm photograph as the page's hero */
export function Desk({ children }: { children: ReactNode }) {
  return <div className="tk-desk">{children}</div>
}

export function Battery() {
  return (
    <svg className="tk-batt" viewBox="0 0 24 12" width="22" height="11" aria-hidden>
      <rect x="0.75" y="0.75" width="19.5" height="10.5" rx="3" fill="none" stroke="#30d158" strokeWidth="1.5" />
      <rect x="2.6" y="2.6" width="13" height="6.8" rx="1.6" fill="#30d158" />
      <rect x="21.4" y="3.8" width="1.8" height="4.4" rx="0.9" fill="#30d158" />
    </svg>
  )
}

/** A concave corner where the island meets the screen edge */
function Ear({ side }: { side: 'l' | 'r' }) {
  return (
    <svg className={`tk-ear tk-ear--${side}`} viewBox="0 0 14 14" width="14" height="14" aria-hidden>
      <path d="M0 0H14A14 14 0 0 0 0 14Z" fill="#000" />
    </svg>
  )
}
