import { Field, Stage, type Act, type Layout } from './kit'
import { Battery, Desk, Island, type IslandBox } from './island'

// A message box is focused on the desktop. Ctrl+Shift+Space: the island widens
// into its recording pill with a live level meter. Ctrl+Shift+Space again: it
// says Transcribing for a beat on this PC, folds back to rest, and the words
// are typed into the box where the caret was. The box clears and the loop
// starts again from the same rest.
//
// Real strings: the shortcut, "Listening", "Transcribing on this PC", the
// sentence typed, the box's placeholder.

type Mode = 'rest' | 'listen' | 'busy'
type S = { mode: Mode; level: number; text: string; focus: boolean }

const SAID = 'Send Ama the slides before Thursday and book the small room.'
const BOX: Record<Mode, IslandBox> = {
  rest: { w: 212, h: 32, r: 16 },
  listen: { w: 300, h: 38, r: 19 },
  busy: { w: 300, h: 38, r: 19 },
}
const GEO: Record<Layout, { w: number; h: number; field: { x: number; y: number; w: number } }> = {
  wide: { w: 520, h: 300, field: { x: 48, y: 128, w: 424 } },
  compact: { w: 358, h: 290, field: { x: 20, y: 120, w: 318 } },
}
const START: S = { mode: 'rest', level: 0, text: '', focus: true }
const POSTER: S = { ...START, text: SAID }

async function script(a: Act<S>) {
  await a.wait(900)
  a.mark('start')
  await a.key('Ctrl+Shift+Space')
  a.put({ mode: 'listen' })
  // A voice: the level rises and falls with the words
  for (let i = 0; i < 22; i++) {
    a.put({ level: 0.25 + Math.abs(Math.sin(i * 1.3)) * 0.75 })
    await a.wait(110)
  }
  a.mark('stop')
  await a.key('Ctrl+Shift+Space')
  a.put({ mode: 'busy', level: 0 })
  await a.wait(900)
  a.put({ mode: 'rest' })
  await a.wait(250)
  a.mark('typed')
  await a.type('text', SAID, { interval: 22 })
  await a.wait(2600)
  a.put({ text: '' })
  await a.wait(900)
}

function Meter({ level }: { level: number }) {
  return (
    <span className="tk-wave" aria-hidden>
      {[8, 16, 12, 16, 8].map((k, i) => <i key={i} style={{ height: 3 + level * k }} />)}
    </span>
  )
}

export default function DictationWalkthrough() {
  return (
    <Stage<S>
      width={520}
      height={300}
      compact={{ below: 460, width: 358, height: 290 }}
      initial={START}
      poster={POSTER}
      script={script}
      keys="bottom"
      className="tk tk-dict"
      fill
    >
      {(s, _rect, layout) => {
        const g = GEO[layout]
        return (
          <Desk>
            <Island box={BOX[s.mode]} cx={g.w / 2} className={`is-${s.mode}`} label="Tuck">
              <div className="tk-rest" data-on={s.mode === 'rest' || undefined}>
                <span className="tk-time">10:24</span>
                <Battery />
              </div>
              <div className="tk-listen" data-on={s.mode === 'listen' || undefined}>
                <span className="tk-dot" />
                <span>Listening</span>
                <Meter level={s.level} />
              </div>
              <div className="tk-listen" data-on={s.mode === 'busy' || undefined}>
                <span className="tk-dot tk-dot--busy" />
                <span>Transcribing on this PC</span>
              </div>
            </Island>
            <div className="tk-message" style={{ left: g.field.x, top: g.field.y, width: g.field.w }}>
              <Field value={s.text} placeholder="Message Ama" focused={s.focus} box={{ 'data-target': 'box' }} />
            </div>
          </Desk>
        )
      }}
    </Stage>
  )
}
