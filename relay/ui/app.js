const PROTOCOL = 1

const el = (id) => document.getElementById(id)
const kind = (message) => (typeof message === 'string' ? message : Object.keys(message)[0])
const body = (message) => (typeof message === 'string' ? {} : Object.values(message)[0])

const token = (() => {
  const fromUrl = new URLSearchParams(location.search).get('t')
  fromUrl && localStorage.setItem('warcry.token', fromUrl)
  fromUrl && history.replaceState({}, '', location.pathname)
  return localStorage.getItem('warcry.token') || '0'.repeat(64)
})()

const state = { socket: null, seq: 0, rigs: new Map(), chime: null, pending: new Set(), alarm: null }

const score = {
  casts: 0,
  caught: 0,
  missed: 0,
  streak: 0,
  reactions: [],
  best: Number(localStorage.getItem('warcry.best') || 0),
  bestStreak: Number(localStorage.getItem('warcry.streak') || 0),
  bitAt: new Map(),
}

const average = (values) =>
  values.length ? values.reduce((a, b) => a + b, 0) / values.length : 0

const stat = (value, label, extra) =>
  `<div class="stat ${extra || ''}"><b>${value}</b><span>${label}</span></div>`

const showScore = (celebrate) => {
  const mean = average(score.reactions)
  el('stats').innerHTML = [
    stat(score.caught, 'caught'),
    stat(score.missed, 'missed', score.missed > 0 ? 'hot' : ''),
    stat(score.streak, 'streak', score.streak >= 3 ? 'hot' : ''),
    stat(
      score.best ? `${(score.best / 1000).toFixed(2)}s` : '--',
      mean ? `best, avg ${(mean / 1000).toFixed(2)}s` : 'reaction',
      celebrate ? 'best' : ''
    ),
  ].join('')
}

const reacted = (rig) => {
  const bit = score.bitAt.get(rig)
  score.bitAt.delete(rig)
  if (!bit) return showScore(false)
  const took = Date.now() - bit
  score.reactions.push(took)
  const record = !score.best || took < score.best
  record && (score.best = took) && localStorage.setItem('warcry.best', String(took))
  showScore(record)
}

const landed = () => {
  score.caught += 1
  score.streak += 1
  score.streak > score.bestStreak &&
    ((score.bestStreak = score.streak), localStorage.setItem('warcry.streak', String(score.streak)))
  showScore(false)
}

const lapsed = (rig) => {
  score.bitAt.has(rig) && ((score.missed += 1), (score.streak = 0))
  score.bitAt.delete(rig)
  showScore(false)
}

const setStatus = (live, text) => {
  el('status').classList.toggle('live', live)
  el('statusText').textContent = text
}

const audio = () => {
  state.chime = state.chime || new (window.AudioContext || window.webkitAudioContext)()
  return state.chime
}

const wavBytes = (freqs) => {
  const rate = 44100
  const tone = 0.16
  const gap = 0.05
  const span = Math.floor(rate * freqs.length * (tone + gap))
  const samples = new Int16Array(span)
  freqs.forEach((freq, index) => {
    const start = Math.floor(rate * index * (tone + gap))
    const len = Math.floor(rate * tone)
    for (let n = 0; n < len; n += 1) {
      const fade = Math.min(1, n / 300) * Math.min(1, (len - n) / 900)
      samples[start + n] = Math.sin((2 * Math.PI * freq * n) / rate) * fade * 0.8 * 32767
    }
  })
  const header = new DataView(new ArrayBuffer(44))
  const tag = (at, text) => [...text].forEach((c, i) => header.setUint8(at + i, c.charCodeAt(0)))
  tag(0, 'RIFF')
  header.setUint32(4, 36 + samples.byteLength, true)
  tag(8, 'WAVEfmt ')
  header.setUint32(16, 16, true)
  header.setUint16(20, 1, true)
  header.setUint16(22, 1, true)
  header.setUint32(24, rate, true)
  header.setUint32(28, rate * 2, true)
  header.setUint16(32, 2, true)
  header.setUint16(34, 16, true)
  tag(36, 'data')
  header.setUint32(40, samples.byteLength, true)
  const bytes = new Uint8Array(44 + samples.byteLength)
  bytes.set(new Uint8Array(header.buffer), 0)
  bytes.set(new Uint8Array(samples.buffer), 44)
  return bytes
}

const dataUrl = (bytes) => {
  let text = ''
  const step = 0x8000
  for (let at = 0; at < bytes.length; at += step) {
    text += String.fromCharCode.apply(null, bytes.subarray(at, at + step))
  }
  return `data:audio/wav;base64,${btoa(text)}`
}

const bell = () => {
  if (!state.bell) {
    state.bell = new Audio(dataUrl(wavBytes([880, 1320, 1760, 1320])))
    state.bell.preload = 'auto'
    state.bell.volume = 1
  }
  return state.bell
}

const sounded = (ok) => {
  el('sound').textContent = ok ? 'sound on' : 'test sound'
  el('sound').classList.toggle('on', ok)
  el('enable').classList.toggle('hidden', ok)
  state.sounding = ok
}

const ring = (urgency) => {
  const player = bell()
  player.currentTime = 0
  player.play().then(() => sounded(true)).catch(() => sounded(false))
  chime(urgency)
}

const chime = (urgency) => {
  const context = audio()
  context.resume()
  const at = context.currentTime
  const tones = urgency >= 3 ? [880, 1320, 1760] : [660]
  tones.forEach((frequency, index) => {
    const osc = context.createOscillator()
    const gain = context.createGain()
    osc.frequency.value = frequency
    osc.type = 'triangle'
    gain.gain.setValueAtTime(0.0001, at + index * 0.12)
    gain.gain.exponentialRampToValueAtTime(0.45, at + index * 0.12 + 0.02)
    gain.gain.exponentialRampToValueAtTime(0.0001, at + index * 0.12 + 0.22)
    osc.connect(gain).connect(context.destination)
    osc.start(at + index * 0.12)
    osc.stop(at + index * 0.12 + 0.24)
  })
}

const biting = () => [...state.rigs.values()].some((made) => made.card.classList.contains('bite'))

const alarm = () => {
  const on = biting()
  document.body.classList.toggle('alarm', on)
  if (on === !!state.alarm) return
  clearInterval(state.alarm)
  state.alarm = on ? setInterval(() => ring(3), 1400) : null
}

const send = (message) => state.socket?.readyState === 1 && state.socket.send(JSON.stringify(message))

const LABELS = { cast: 'cast', reel: 'reel', casting: 'casting...', reeling: 'reeling...' }
const WAITING = ['casting', 'reeling']
const GIVE_UP = 3000

const press = (rig) => {
  const made = state.rigs.get(rig)
  if (!made || made.button.disabled) return
  const reeling = made.mode === 'reel'
  state.seq += 1
  state.pending.add(state.seq)
  send({
    press: { rig, slot: reeling ? made.reel : made.cast, seq: state.seq, at_ms: Date.now() },
  })
  made.card.classList.remove('bite')
  reeling && reacted(rig)
  mode(made, reeling ? 'reeling' : 'casting')
  clearTimeout(made.revert)
  made.revert = setTimeout(() => mode(made, 'cast'), GIVE_UP)
  alarm()
}

const slotFor = (rig, label) => {
  const match = rig.actions.find((action) => action.label === label)
  return (match || rig.actions[0] || { slot: 0 }).slot
}

const tile = (rig) => {
  const card = document.createElement('section')
  card.className = 'rig'
  const heading = document.createElement('h2')
  heading.textContent = rig.label
  const state_line = document.createElement('div')
  state_line.className = 'state'
  state_line.textContent = 'idle'
  const ring = document.createElement('div')
  ring.className = 'ring'
  const fill = document.createElement('span')
  ring.append(fill)
  const button = document.createElement('button')
  button.className = 'reel'
  button.type = 'button'
  const made = {
    card,
    state_line,
    fill,
    button,
    deadline: 0,
    cast: slotFor(rig, 'cast'),
    reel: slotFor(rig, 'reel'),
    mode: 'cast',
  }
  button.addEventListener('pointerdown', (event) => {
    event.preventDefault()
    press(rig.id)
  })
  card.append(heading, state_line, ring, button)
  mode(made, 'cast')
  return made
}

const mode = (made, next) => {
  made.mode = next
  made.button.textContent = LABELS[next] || next
  made.button.dataset.label = next
  made.button.disabled = WAITING.includes(next)
}

const render = (rigs) => {
  const host = el('rigs')
  host.textContent = ''
  state.rigs.clear()
  rigs.forEach((rig) => {
    const made = tile(rig)
    state.rigs.set(rig.id, made)
    host.append(made.card)
  })
  el('empty').classList.toggle('hidden', rigs.length > 0)
}

const describe = (rig_state) => {
  const name = kind(rig_state)
  const fields = body(rig_state)
  const words = {
    idle: 'idle',
    casting: 'fishing',
    bite: 'FISH ON',
    caught: 'caught',
    expired: 'cast lapsed',
    bags_full: 'bags full',
    offline: 'offline',
  }
  return { name, label: words[name] || name, fields }
}

const applyState = (rig, rig_state) => {
  const made = state.rigs.get(rig)
  if (!made) return
  const { name, label, fields } = describe(rig_state)
  name === 'casting' && ((score.casts += 1), showScore(false))
  name === 'expired' && lapsed(rig)
  made.state_line.textContent = label
  made.card.classList.toggle('bite', name === 'bite')
  made.card.classList.toggle('bags', name === 'bags_full')
  made.deadline = name === 'casting' ? Date.now() + fields.ends_in_ms : 0
  name !== 'casting' && (made.fill.style.width = '0%')
  clearTimeout(made.revert)
  mode(made, settled(name))
  alarm()
}

const settled = (name) => {
  if (name === 'bite') return 'reel'
  if (name === 'casting') return 'casting'
  return 'cast'
}

const countdown = () => {
  const now = Date.now()
  state.rigs.forEach((made) => {
    const left = made.deadline - now
    made.fill.style.width = left > 0 ? `${Math.min(100, (left / 30000) * 100)}%` : '0%'
    made.deadline > 0 && left > 0 && (made.state_line.textContent = `fishing ${Math.ceil(left / 1000)}s`)
    if (made.deadline > 0 && left <= 0) {
      made.deadline = 0
      made.card.classList.remove('bite')
      made.state_line.textContent = 'cast lapsed'
      mode(made, 'cast')
      alarm()
    }
  })
}

const alerted = (rig, alert) => {
  const name = kind(alert)
  const urgency = name === 'bite' ? 3 : name === 'bags_full' ? 2 : 1
  name !== 'caught' && ring(urgency)
  name === 'caught' && landed()
  const made = state.rigs.get(rig)
  if (!made || name !== 'bite') return
  score.bitAt.set(rig, Date.now())
  made.card.classList.add('bite')
  mode(made, 'reel')
  alarm()
}

const handlers = {
  welcome: (fields) => {
    setStatus(true, 'connected')
    render(fields.rigs)
  },
  denied: (fields) => setStatus(false, `refused: ${kind(fields.reason)}`),
  rigs: (fields) => render(fields),
  state: (fields) => applyState(fields.rig, fields.state),
  alert: (fields) => alerted(fields.rig, fields.alert),
  pressed: (fields) => {
    state.pending.delete(fields.seq)
    el('latency').textContent = fields.ok ? 'sent' : 'refused'
    const made = state.rigs.get(fields.rig)
    made && !fields.ok && (clearTimeout(made.revert), mode(made, 'cast'))
  },
  pong: () => {},
}

const connect = () => {
  const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
  const socket = new WebSocket(`${scheme}://${location.host}/ws`)
  state.socket = socket
  socket.addEventListener('open', () => {
    setStatus(false, 'authenticating')
    send({ auth: { token, protocol: PROTOCOL, client: 'web' } })
  })
  socket.addEventListener('message', (event) => {
    const message = JSON.parse(event.data)
    const handler = handlers[kind(message)]
    handler && handler(body(message))
  })
  socket.addEventListener('close', () => {
    setStatus(false, 'reconnecting')
    setTimeout(connect, 1200)
  })
  socket.addEventListener('error', () => socket.close())
}

const wake = async () => {
  const button = el('wake')
  const held = button.classList.contains('on')
  held && state.lock && (await state.lock.release(), (state.lock = null))
  if (held) return button.classList.remove('on')
  try {
    state.lock = await navigator.wakeLock.request('screen')
    button.classList.add('on')
    state.lock.addEventListener('release', () => button.classList.remove('on'))
  } catch {
    button.textContent = 'screen lock unavailable'
  }
}

el('wake').addEventListener('click', wake)
const unlock = () => {
  audio().resume()
  state.sounding || ring(3)
}
document.addEventListener('pointerdown', unlock)
el('enable').addEventListener('pointerdown', (event) => {
  event.preventDefault()
  unlock()
})
el('sound').addEventListener('pointerdown', (event) => {
  event.preventDefault()
  ring(3)
})
setInterval(countdown, 200)
showScore(false)
connect()
