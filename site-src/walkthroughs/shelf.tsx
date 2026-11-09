import { Stage, type Act, type Layout, type Point } from './kit'
import { Battery, Desk, Island, type IslandBox } from './island'

// A photo on the desktop is dragged up onto the resting island. The island
// opens into its drop zone as the file comes close, says "Release to add", and
// on the drop the photo lands as a tile on the shelf beside the file already
// there. The island folds back to rest and the photo is back on the desktop for
// the next loop, which is where it began.
//
// Real strings: the time, "Release to add", the two file names and sizes.

type Mode = 'rest' | 'drop' | 'shelf'
type S = { mode: Mode; carrying: boolean; at: Point; landed: boolean; onDesk: boolean }

const BOX: Record<Mode, IslandBox> = {
  rest: { w: 212, h: 32, r: 16 },
  drop: { w: 400, h: 168, r: 32 },
  shelf: { w: 400, h: 168, r: 32 },
}
const GEO: Record<Layout, { w: number; h: number; file: Point; scale: number }> = {
  wide: { w: 520, h: 300, file: { x: 64, y: 196 }, scale: 1 },
  compact: { w: 358, h: 290, file: { x: 40, y: 196 }, scale: 0.84 },
}
const START: S = { mode: 'rest', carrying: false, at: { x: 0, y: 0 }, landed: false, onDesk: true }
const POSTER: S = { ...START, mode: 'shelf', landed: true, onDesk: false }

async function script(a: Act<S>) {
  await a.wait(700)
  const icon = a.rect('desk-file')
  const grab = { x: icon.x + icon.width / 2, y: icon.y + icon.height * 0.4 }
  await a.move(grab)
  await a.wait(200)
  a.put({ carrying: true, onDesk: false, at: grab })
  const island = a.rect('island')
  const target = { x: island.x + island.width / 2, y: island.y + 20 }
  let opened = false
  await a.drag(target, (at) => {
    // The drop zone opens as soon as the file comes within reach of the island
    if (!opened && at.y < island.y + 130) { opened = true; a.put({ mode: 'drop' }) }
    a.put({ at })
  }, { duration: 1400 })
  a.mark('drop')
  await a.wait(500)
  a.put({ carrying: false, mode: 'shelf', landed: true })
  await a.wait(2600)
  a.mark('rest')
  a.put({ mode: 'rest' })
  void a.move({ x: GEO[a.layout].w - 60, y: GEO[a.layout].h - 40 }, { duration: 700 })
  await a.wait(900)
  await a.snap({ landed: false })
  a.put({ onDesk: true })
  await a.wait(800)
}

function Photo() {
  return <img className="tk-thumb" src="img/thumb-beach.png" alt="" width="80" height="50" />
}

export default function ShelfWalkthrough() {
  return (
    <Stage<S>
      width={520}
      height={300}
      compact={{ below: 460, width: 358, height: 290 }}
      initial={START}
      poster={POSTER}
      script={script}
      cursor={{ at: { wide: { x: 460, y: 260 }, compact: { x: 300, y: 250 } } }}
      className="tk tk-shelf"
      fill
    >
      {(s, _rect, layout) => {
        const g = GEO[layout]
        const box = { w: BOX[s.mode].w * g.scale, h: BOX[s.mode].h * g.scale, r: BOX[s.mode].r }
        return (
          <Desk>
            <Island box={box} cx={g.w / 2} className={`is-${s.mode}`} label="Tuck">
              <div className="tk-rest" data-on={s.mode === 'rest' || undefined}>
                <span className="tk-time">10:24</span>
                <Battery />
              </div>
              <div className="tk-drop" data-on={s.mode === 'drop' || undefined}>
                <svg viewBox="0 0 24 24" width="28" height="28" aria-hidden><path d="M4 13.5V18a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-4.5M4 13.5 6.5 6h11L20 13.5M4 13.5h4.5l1.2 2.2h4.6l1.2-2.2H20M12 3v7m0 0-2.6-2.6M12 10l2.6-2.6" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
                <span>Release to add</span>
              </div>
              <div className="tk-tiles" data-on={s.mode === 'shelf' || undefined}>
                <figure className="tk-tile">
                  <img src="img/thumb-route.png" alt="" width="80" height="50" />
                  <figcaption>weekend-route.png<small>1.8 MB</small></figcaption>
                </figure>
                <figure className="tk-tile tk-tile--new" data-on={s.landed || undefined}>
                  <img src="img/thumb-beach.png" alt="" width="80" height="50" />
                  <figcaption>beach.jpg<small>2.4 MB</small></figcaption>
                </figure>
              </div>
            </Island>

            <div className="tk-file" data-target="desk-file" data-away={!s.onDesk || undefined} style={{ left: g.file.x, top: g.file.y }}>
              <Photo />
              <span>beach.jpg</span>
            </div>
            <div className="tk-ghost" data-on={s.carrying || undefined} style={{ transform: `translate(${s.at.x - 40}px, ${s.at.y - 20}px)` }} aria-hidden>
              <Photo />
            </div>
          </Desk>
        )
      }}
    </Stage>
  )
}
