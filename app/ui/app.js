const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event

const $ = (id) => document.getElementById(id)
const el = (tag, props, ...children) => {
  const node = Object.assign(document.createElement(tag), props)
  node.append(...children)
  return node
}
const basename = (path) => String(path).split(/[\\/]/).pop()
const clock = () => new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })

let state = null

const toast = (message) => {
  const box = $('toast')
  box.textContent = message
  box.hidden = false
  clearTimeout(toast.timer)
  toast.timer = setTimeout(() => { box.hidden = true }, 5000)
}

const ask = (title, body, yes) => new Promise((answer) => {
  const box = $('ask')
  $('ask-title').textContent = title
  $('ask-body').textContent = body
  $('ask-yes').textContent = yes || 'Delete'
  const close = (said) => {
    answer(said)
    box.close()
  }
  $('ask-yes').onclick = () => close(true)
  $('ask-no').onclick = () => close(false)
  box.onclose = () => answer(false)
  box.showModal()
})

const call = (command, args) => invoke(command, args).catch((error) => {
  toast(String(error))
  throw error
})

const fill = (select, items, selected) => {
  select.replaceChildren(...items.map(([value, label]) => el('option', { value, textContent: label, selected: value === selected })))
}

const STATUS = {
  busy: 'Starting…',
  off: 'Off',
  on: 'Listening',
  paused: 'Paused',
}

let targets = []

const windowLabel = (w) => `${w.title || w.process} — pid ${w.pid}`

const KEEP = 'keep'

const savedLabel = (target) => `${target.title || target.process || 'saved window'} \u2014 ${
  target.pid ? 'not running' : 'by title, whichever is open'}`

const chosenValue = (target) => {
  if (!target) return ''
  return target.pid && targets.some((w) => w.pid === target.pid) ? String(target.pid) : KEEP
}

const targetOptions = (target) => [
  ['', 'whatever window is focused'],
  ...targets.map((w) => [String(w.pid), windowLabel(w)]),
  ...(target ? [[KEEP, savedLabel(target)]] : []),
]

const renderTarget = (target) => {
  const pick = $('target-pick')
  if (document.activeElement !== pick) fill(pick, targetOptions(target), chosenValue(target))
  $('target-mode').value = (target && target.mode) || 'focus_restore'
  $('target').replaceChildren(...[target && (target.found
    ? el('span', { className: 'found', textContent: `\u2713 ${target.found}` })
    : el('span', { className: 'missing', textContent: 'not open right now' }))].filter(Boolean))
}

const loadTargets = () => call('target_windows').then((found) => {
  targets = found || []
  if (state) renderTarget(state.target)
})

let editing = null

const draftFields = {
  name: 'd-name',
  phrase: 'd-phrase',
  keys: 'd-keys',
  hotkey: 'd-hotkey',
}

const MAC = (navigator.platform || '').toLowerCase().includes('mac')

const CAP_LABELS = MAC
  ? { ctrl: '⌃', alt: '⌥', shift: '⇧', cmd: '⌘' }
  : { ctrl: 'Ctrl', alt: 'Alt', shift: 'Shift', cmd: 'Win' }

const CAP_GLYPHS = {
  space: 'Space', enter: '↩', tab: '⇥', escape: 'Esc',
  backspace: '⌫', delete: 'Del', up: '↑', down: '↓',
  left: '←', right: '→', pageup: 'PgUp', pagedown: 'PgDn',
  home: 'Home', end: 'End',
}

const capLabel = (part) => CAP_LABELS[part] || CAP_GLYPHS[part]
  || (part.length <= 3 ? part.toUpperCase() : part)

const keycaps = (text, title, extra) => el('span', {
  className: `caps${extra ? ` ${extra}` : ''}`,
  title: title || '',
}, ...String(text).split('+').filter(Boolean).map((part) => el('kbd', { textContent: capLabel(part) })))

const CODE_NAMES = {
  Space: 'space', Enter: 'enter', Tab: 'tab', Escape: 'escape',
  Backspace: 'backspace', Delete: 'delete', ArrowUp: 'up', ArrowDown: 'down',
  ArrowLeft: 'left', ArrowRight: 'right', Home: 'home', End: 'end',
  PageUp: 'pageup', PageDown: 'pagedown', Minus: '-', Equal: '=',
  BracketLeft: '[', BracketRight: ']', Backslash: '\\', Semicolon: ';',
  Quote: "'", Comma: ',', Period: '.', Slash: '/', Backquote: '`',
}

const baseKey = (event) => {
  const letter = /^Key([A-Z])$/.exec(event.code)
  if (letter) return letter[1].toLowerCase()
  const digit = /^(?:Digit|Numpad)(\d)$/.exec(event.code)
  if (digit) return digit[1]
  const fkey = /^F(\d{1,2})$/.exec(event.code)
  if (fkey) return `f${fkey[1]}`
  return CODE_NAMES[event.code] || null
}

const comboOf = (event) => {
  const base = baseKey(event)
  if (!base) return null
  const parts = []
  if (event.ctrlKey) parts.push('ctrl')
  if (event.altKey) parts.push('alt')
  if (event.shiftKey) parts.push('shift')
  if (event.metaKey) parts.push('cmd')
  parts.push(base)
  return parts.join('+')
}

const repaints = []

const keyButton = (start, hint, onSet) => {
  const slot = el('span', { className: 'slot' })
  const field = el('button', { type: 'button', className: 'keyfield' }, slot)
  const wipe = el('button', { type: 'button', className: 'keyclear', textContent: '×', title: 'clear' })
  let value = start || ''
  const paint = () => {
    field.classList.remove('arming')
    slot.replaceChildren(value ? keycaps(value) : el('em', { textContent: hint }))
    wipe.hidden = !value
  }
  const set = (next) => {
    const changed = next !== value
    value = next
    paint()
    if (changed) onSet(next)
  }
  const onKey = (event) => {
    event.preventDefault()
    event.stopPropagation()
    document.removeEventListener('keydown', onKey, true)
    set((event.code === 'Escape' ? null : comboOf(event)) || value)
  }
  field.addEventListener('click', () => {
    field.classList.add('arming')
    slot.replaceChildren(el('em', { className: 'waiting', textContent: 'press a key…' }))
    document.addEventListener('keydown', onKey, true)
  })
  wipe.addEventListener('click', () => set(''))
  paint()
  return el('span', { className: 'keyrow' }, field, wipe)
}

const armCapture = (id, hint) => {
  const input = $(id)
  if (!input) return
  input.hidden = true
  let shown = keyButton(input.value, hint, (value) => (input.value = value))
  input.after(shown)
  repaints.push(() => {
    const next = keyButton(input.value, hint, (value) => (input.value = value))
    shown.replaceWith(next)
    shown = next
  })
}

const showEditor = (index, draft) => {
  editing = index
  Object.entries(draftFields).forEach(([key, id]) => ($(id).value = draft[key] || ''))
  $('d-active').checked = !!draft.always_active
  $('d-pad').checked = !!draft.pad
  repaints.forEach((paint) => paint())
  $('d-head').textContent = index === null ? 'New command' : `Edit ${draft.name || 'this command'}`
  $('d-error').textContent = ''
  $('editor').showModal()
  $('d-name').focus()
}

const hideEditor = () => {
  editing = null
  $('editor').close()
}

const saveDraft = () => {
  const draft = Object.fromEntries(
    Object.entries(draftFields).map(([key, id]) => [key, $(id).value.trim()])
  )
  draft.always_active = $('d-active').checked
  draft.pad = $('d-pad').checked
  invoke('edit_command', { index: editing, draft })
    .then(hideEditor)
    .then(refresh)
    .catch((error) => ($('d-error').textContent = String(error)))
}

const removeCommand = (c) => ask(
  'Delete this command?',
  `"${c.name}" will go from this profile, along with everything you can say to run it.`,
).then((yes) => yes && call('remove_command', { index: c.index }).then(refresh))

const haystack = (c) => [c.name, c.draft.keys, c.hotkey, ...c.phrases]
  .filter(Boolean)
  .join(' ')
  .toLowerCase()

const phraseCell = (c) => el('td', { className: 'say' },
  ...c.phrases.slice(0, 2).map((phrase) => el('span', { textContent: `“${phrase}”` })),
  c.more || c.phrases.length > 2
    ? el('span', { className: 'dim', textContent: `+${c.more + Math.max(0, c.phrases.length - 2)} more` })
    : '')

const setKey = (c, field, value) => call('edit_command', {
  index: c.index,
  draft: { ...c.draft, [field]: value },
}).then(refresh)

const keyCell = (c, field, hint) => el('td', {}, c.editable
  ? keyButton(c.draft[field], hint, (value) => setKey(c, field, value))
  : (c.draft[field]
    ? keycaps(c.draft[field], 'open the file to change this')
    : el('span', { className: 'dim', textContent: '—' })))

const commandRow = (c) => el('tr', { hay: haystack(c) },
  el('td', {}, el('strong', { textContent: c.name }),
    c.actions > 1 ? el('span', { className: 'dim', textContent: ` ${c.actions} actions` }) : ''),
  phraseCell(c),
  keyCell(c, 'keys', 'set a key'),
  keyCell(c, 'hotkey', 'none'),
  el('td', { className: 'row-actions' },
    el('button', {
      className: c.pad ? 'on' : '',
      title: c.pad ? 'on your phone' : 'add to your phone',
      textContent: 'Phone',
      onclick: () => call('set_pad', { index: c.index, on: !c.pad }).then(refresh),
    }),
    c.editable
      ? el('button', { textContent: 'Edit', onclick: () => showEditor(c.index, c.draft) })
      : el('button', { disabled: true, title: 'too detailed to edit here, open the file instead', textContent: 'Edit' }),
    el('button', { textContent: 'Delete', onclick: () => removeCommand(c) }),
  ),
)

const applySearch = () => {
  const needle = ($('command-search').value || '').trim().toLowerCase()
  const rows = [...$('commands').children]
  const shown = rows.filter((row) => {
    const hit = !needle || (row.hay || '').includes(needle)
    row.hidden = !hit
    return hit
  })
  $('commands-empty').hidden = shown.length > 0 || rows.length === 0
  $('command-count').textContent = needle
    ? `${shown.length} of ${rows.length}`
    : `${rows.length} command${rows.length === 1 ? '' : 's'}`
  $('command-tally').textContent = rows.length || ''
}

const renderCommands = (commands) => {
  $('commands').replaceChildren(...commands.map(commandRow))
  applySearch()
}

const flavorName = (label) => {
  const tail = label.split(' ').pop() || label
  const bare = tail.replace(/^_|_$/g, '').replace(/_/g, ' ')
  return bare.charAt(0).toUpperCase() + bare.slice(1)
}

const addonRow = (a) => el('div', { className: 'rw' },
  el('span', { textContent: flavorName(a.label) }),
  el('span', {
    className: a.installed === a.bundled ? 'found' : 'meta',
    textContent: a.installed === a.bundled ? 'up to date' : (a.installed ? `v${a.installed}` : 'not installed'),
  }),
)

const installAddon = (button, wanted) => {
  button.disabled = true
  Promise.all(wanted.map((a) => invoke('install_addon', { flavor: a.flavor })))
    .then(() => toast('Addon installed. Restart World of Warcraft.'))
    .catch((error) => toast(String(error)))
    .finally(renderAddon)
}

const renderAddon = () => call('wow_addon').then((installs) => {
  const box = $('addon')
  const list = installs || []
  $('addon-version').textContent = list.length ? `v${list[0].bundled}` : ''
  if (!list.length) {
    return box.replaceChildren(el('div', { className: 'rw' }, el('span', { className: 'meta', textContent: 'No World of Warcraft found' })))
  }
  const stale = list.filter((a) => a.installed !== a.bundled)
  box.replaceChildren(...list.map(addonRow), el('div', { className: 'rw' },
    el('span', { textContent: stale.length ? 'An update is waiting' : 'Everything is current' }),
    el('div', { className: 'ctl' }, el('button', {
      className: stale.length ? 'primary' : 'ghost',
      textContent: stale.length ? `Install into ${stale.length} ${stale.length === 1 ? 'client' : 'clients'}` : 'Reinstall',
      onclick: (event) => installAddon(event.currentTarget, stale.length ? stale : list),
    })),
  ))
})

const seatRow = (seat) => el('li', {},
  el('b', { textContent: seat.label }),
  el('span', { className: 'meta', textContent: seat.deck ? 'your buttons' : `pid ${seat.pid}` }),
  el('div', { className: 'phrases' }, ...seat.buttons.map((b) => el('span', { textContent: b }))),
)

const renderPhone = (s) => {
  const live = s.companion
  const names = s.commands.map((c) => [c.name, c.name])
  $('phone-count').textContent = s.commands.filter((c) => c.pad).length || ''
  $('phone-state').textContent = live.running
    ? `on, hearing ${live.hearing} window${live.hearing === 1 ? '' : 's'}`
    : 'off'
  $('foot-deck').textContent = live.running ? `phone deck on · ${live.url || ''}` : 'phone deck off'
  $('phone-power').textContent = live.running ? 'Turn off' : 'Turn on'
  $('phone-power').className = live.running ? 'ghost' : 'primary'
  $('phone-pair').hidden = !live.running
  $('phone-copy').hidden = !live.running
  $('phone-qr').innerHTML = live.qr || ''
  $('phone-url').textContent = live.url || ''
  $('phone-seats').replaceChildren(...(live.seats || []).map(seatRow))
  fill($('phone-cast'), [['', 'just press a key'], ...names], s.deck.cast || '')
  fill($('phone-reel'), [['', 'just press a key'], ...names], s.deck.reel || '')
  const keep = (id, value) => document.activeElement === $(id) || ($(id).value = value)
  keep('phone-key', s.deck.key || '')
  keep('phone-port', s.deck.port)
  keep('phone-advertise', s.deck.advertise || '')
  $('phone-enabled').checked = s.deck.enabled
  $('phone-trust').checked = s.deck.trust
  $('phone-speak').checked = s.deck.speak
}

const footTarget = (t) => [
  t.title || t.process || 'window',
  t.pid && `pid ${t.pid}`,
  !t.found && 'not open',
].filter(Boolean).join(' · ')

const render = (s) => {
  state = s
  if (s.failure) toast(s.failure)
  const mode = [s.busy && 'busy', !s.running && 'off', s.listening && 'on', 'paused'].find(Boolean)
  $('status').className = `pill ${mode}`
  $('status').querySelector('b').textContent = STATUS[mode]
  $('ptt').hidden = !s.push_to_talk
  $('ptt').textContent = `Hold ${s.push_to_talk} to talk`
  $('listening').checked = s.running && s.listening
  $('listening').disabled = s.busy || !s.profile
  fill($('profile'), s.profiles.map((p) => [p, basename(p)]), s.profile)
  $('profile').hidden = !s.profiles.length
  $('edit').disabled = !s.profile
  $('profile-error').hidden = !s.profile_error
  $('profile-error').textContent = s.profile_error || ''
  renderTarget(s.target)
  $('foot-target').textContent = s.target ? footTarget(s.target) : 'no target window'
  fill($('device'), [['', 'System default'], ...s.devices.map((d) => [d, d])], s.config.device || '')
  fill($('model'), s.models.map((m) => [m.name, `${m.name} · ${m.about}`]), s.model)
  const current = s.models.find((m) => m.name === s.model)
  $('model-action').hidden = !current || current.downloaded || !$('progress').hidden
  $('auto-start').checked = s.config.auto_start
  $('welcome').hidden = Boolean(s.profile)
  $('feed-empty').hidden = !s.profile || $('feed').children.length > 0
  renderCommands(s.commands)
  renderPhone(s)
}

const refresh = () => call('snapshot').then(render)

const saveConfig = (patch) => call('set_config', { config: { ...state.config, ...patch } }).then(refresh)

const saveDeck = (patch) => call('set_deck', { deck: { ...state.deck, ...patch } }).then(refresh)

const URGENT = 3

const alarm = (urgency) => {
  if (urgency < URGENT) return
  alarm.context = alarm.context || new AudioContext()
  const context = alarm.context
  context.resume()
  const at = context.currentTime
  ;[880, 1320, 1760].forEach((frequency, index) => {
    const osc = context.createOscillator()
    const gain = context.createGain()
    osc.frequency.value = frequency
    osc.type = 'triangle'
    gain.gain.setValueAtTime(0.0001, at + index * 0.12)
    gain.gain.exponentialRampToValueAtTime(0.4, at + index * 0.12 + 0.02)
    gain.gain.exponentialRampToValueAtTime(0.0001, at + index * 0.12 + 0.24)
    osc.connect(gain).connect(context.destination)
    osc.start(at + index * 0.12)
    osc.stop(at + index * 0.12 + 0.26)
  })
}

const addPhoneFeed = (notice) => {
  const feed = $('phone-feed')
  feed.prepend(el('li', { className: notice.urgency >= URGENT ? 'fired' : 'miss' },
    el('time', { textContent: clock() }),
    el('div', { className: 'said' }, notice.text),
    el('span', { className: 'chip', textContent: notice.kind }),
  ))
  Array.from(feed.children).slice(60).forEach((child) => child.remove())
  $('phone-feed-empty').hidden = true
  alarm(notice.urgency)
  if (notice.kind === 'attached' || notice.kind === 'named' || notice.kind === 'deaf') refresh()
}

const addFeed = (note) => {
  const percent = Math.round((note.score || 0) * 100)
  const chip = note.fired ? `▶ ${note.command}` : 'no match'
  const detail = note.command && !note.fired ? `closest: ${note.command} (${percent}%)` : ''
  const shown = [note.millis && `${note.millis} ms`, note.fired && `${percent}%`, detail].filter(Boolean).join(' · ')
  const item = el('li', { className: note.fired ? 'fired' : 'miss' },
    el('time', { textContent: clock() }),
    el('div', { className: 'said' }, `“${note.text}”`, el('small', { textContent: shown })),
    el('span', { className: 'chip', textContent: chip }),
  )
  const feed = $('feed')
  feed.prepend(item)
  Array.from(feed.children).slice(100).forEach((child) => child.remove())
  $('feed-empty').hidden = true
}

const showProgress = (note) => {
  const box = $('progress')
  const done = note.total > 0 && note.read >= note.total
  box.hidden = done
  $('model-action').hidden = true
  box.querySelector('.fill').style.width = `${(note.read / Math.max(note.total, 1)) * 100}%`
  box.querySelector('span').textContent = `Downloading ${note.model} · ${note.read >> 20} / ${note.total >> 20} MB`
  if (done) refresh()
}

const appendLog = (line) => {
  const log = $('log')
  log.append(el('div', { className: line.level, textContent: `${clock()}  ${line.level.padEnd(5)}  ${line.message}` }))
  Array.from(log.children).slice(0, -500).forEach((child) => child.remove())
  log.parentElement.scrollTop = log.parentElement.scrollHeight
}

const NOTES = {
  heard: addFeed,
  fired: (note) => note.source === 'hotkey' && addFeed({ text: 'hotkey', command: note.command, fired: true, score: 1 }),
  downloading: showProgress,
  ready: refresh,
  stopped: refresh,
  listening: refresh,
  reloaded: refresh,
}

listen('note', ({ payload }) => (NOTES[payload.kind] || (() => {}))(payload))
listen('companion', ({ payload }) => addPhoneFeed(payload))
listen('log', ({ payload }) => appendLog(payload))
listen('changed', refresh)
listen('failure', refresh)

const setListening = (on) => {
  if (on && !state.running) {
    render({ ...state, busy: true })
    return call('start').then(() => call('set_listening', { on: true })).finally(refresh)
  }
  call('set_listening', { on }).then(refresh)
}

const retarget = (pid, mode) => call('set_target', { pid, mode }).then(refresh)

$('listening').addEventListener('change', (e) => setListening(e.target.checked))
$('profile').addEventListener('change', (e) => saveConfig({ profile: e.target.value }))
$('target-pick').addEventListener('change', (e) => e.target.value === KEEP
  || retarget(Number(e.target.value) || null, $('target-mode').value))
$('target-mode').addEventListener('change', (e) => retarget(state.target && state.target.pid, e.target.value))
$('device').addEventListener('change', (e) => saveConfig({ device: e.target.value || null }))
$('model').addEventListener('change', (e) => saveConfig({ model: e.target.value }))
$('auto-start').addEventListener('change', (e) => saveConfig({ auto_start: e.target.checked }))
$('download').addEventListener('click', () => call('download_model', { name: state.model }).finally(refresh))
$('edit').addEventListener('click', () => call('open_profile'))
$('command-search').addEventListener('input', applySearch)
armCapture('d-keys', 'click, then press the key')
armCapture('d-hotkey', 'none')
$('add-command').addEventListener('click', () => showEditor(null, {}))
$('d-save').addEventListener('click', saveDraft)
$('d-cancel').addEventListener('click', hideEditor)
$('folder').addEventListener('click', () => call('open_folder'))

const profileError = (error) => {
  $('p-error').hidden = !error
  $('p-error').textContent = error || ''
  $('p-error').className = error ? 'missing' : 'note'
}

const profileCall = (command, args) => invoke(command, args)
  .then(() => profileError(''))
  .then(refresh)
  .catch((error) => profileError(String(error)))

$('p-create').addEventListener('click', () => profileCall('create_profile', {
  name: $('p-new').value,
  example: $('p-from').value,
}).then(() => ($('p-new').value = '')))
$('p-rename-go').addEventListener('click', () => profileCall('rename_profile', {
  name: $('p-rename').value,
}).then(() => ($('p-rename').value = '')))
$('p-delete').addEventListener('click', () => ask(
  'Delete this profile?',
  `${basename(state.profile || '')} and every command in it will be removed from your Warcry folder. This cannot be undone.`,
  'Delete the profile',
).then((yes) => yes && profileCall('delete_profile')))
$('welcome-default').addEventListener('click', () => profileCall('create_profile', {
  name: 'default',
  example: 'default',
}))

$('phone-power').addEventListener('click', (event) => {
  event.currentTarget.disabled = true
  const command = state.companion.running ? 'companion_stop' : 'companion_start'
  call(command).finally(() => {
    event.currentTarget.disabled = false
    refresh()
  })
})
$('phone-copy').addEventListener('click', () => {
  navigator.clipboard.writeText(state.companion.url || '').then(() => toast('Link copied'))
})
$('phone-cast').addEventListener('change', (e) => saveDeck({ cast: e.target.value || null }))
$('phone-reel').addEventListener('change', (e) => saveDeck({ reel: e.target.value || null }))
$('phone-key').addEventListener('change', (e) => saveDeck({ key: e.target.value.trim() || "'" }))
$('phone-port').addEventListener('change', (e) => saveDeck({ port: Number(e.target.value) || 8137 }))
$('phone-advertise').addEventListener('change', (e) => saveDeck({ advertise: e.target.value.trim() || null }))
$('phone-enabled').addEventListener('change', (e) => saveDeck({ enabled: e.target.checked }))
$('phone-trust').addEventListener('change', (e) => saveDeck({ trust: e.target.checked }))
$('phone-speak').addEventListener('change', (e) => saveDeck({ speak: e.target.checked }))

$('try').addEventListener('submit', (e) => {
  e.preventDefault()
  const text = $('try-text').value.trim()
  if (!text) return
  call('try_phrase', { text }).then((result) => { $('try-result').textContent = result })
})

const RAIL = [...document.querySelectorAll('.rail button[data-tab]')]
RAIL.forEach((button) => button.addEventListener('click', () => {
  RAIL.forEach((b) => b.classList.toggle('active', b === button))
  document.querySelectorAll('.tab').forEach((tab) => { tab.hidden = tab.id !== `tab-${button.dataset.tab}` })
  $('view').textContent = button.querySelector('em').textContent
}))

const SLASH = [
  ['#showtooltip', 'Show the icon and tooltip of whatever this macro would cast right now. Put it on the first line.'],
  ['/cast', 'Cast a spell by name. /cast [@party1,help,nodead] Healing Touch'],
  ['/castsequence', 'Cast spells in order, one per press. /castsequence reset=10 Moonfire, Wrath'],
  ['/castrandom', 'Cast one of several spells at random.'],
  ['/use', 'Use an item by name, bag slot or equipment slot. /use 13 is your first trinket.'],
  ['/target', 'Target a unit by name or unit id. /target [@focus]'],
  ['/focus', 'Set your focus to the current target. /focus [@mouseover]'],
  ['/clearfocus', 'Drop your focus.'],
  ['/assist', 'Target whatever your target is targeting.'],
  ['/startattack', 'Begin auto-attack without a new swing timer.'],
  ['/stopattack', 'Stop auto-attack.'],
  ['/stopcasting', 'Cancel the spell you are casting. Put it before a cast you want to cut in.'],
  ['/stopmacro', 'Stop here and ignore the rest of this macro. /stopmacro [nocombat]'],
  ['/cancelaura', 'Remove one of your own buffs by name. /cancelaura Bear Form'],
  ['/cancelform', 'Leave your current shapeshift form.'],
  ['/dismount', 'Get off your mount.'],
  ['/equip', 'Equip an item by name.'],
  ['/equipslot', 'Equip an item into a numbered slot. /equipslot 16 Thunderfury'],
  ['/petattack', 'Send your pet at the target.'],
  ['/petfollow', 'Call your pet back to you.'],
  ['/petpassive', 'Put your pet on passive.'],
  ['/petdefensive', 'Put your pet on defensive.'],
  ['/click', 'Press a button on the interface by its frame name. /click WarcryBeaconButton'],
  ['/run', 'Run Lua. It cannot cast spells or do anything the game protects.'],
  ['/say', 'Say something out loud.'],
  ['/party', 'Say something in party chat.'],
  ['/raid', 'Say something in raid chat.'],
  ['/emote', 'Perform a custom emote.'],
]

const STATES = [
  ['@party1', 'Act on party member 1 without changing your target. Also party2-4, raid1-40.'],
  ['@player', 'Act on yourself.'],
  ['@pet', 'Act on your pet.'],
  ['@focus', 'Act on your focus.'],
  ['@mouseover', 'Act on whatever is under your cursor, including a unit frame.'],
  ['@target', 'Act on your current target.'],
  ['@targettarget', "Act on your target's target."],
  ['@none', 'Act on nothing, which turns a cast into a ground-targeted one.'],
  ['help', 'The unit is friendly and you can help it.'],
  ['harm', 'The unit is hostile and you can attack it.'],
  ['exists', 'The unit exists at all.'],
  ['dead', 'The unit is dead.'],
  ['combat', 'You are in combat.'],
  ['mod', 'Any modifier key is held.'],
  ['mod:shift', 'Shift is held. Also mod:ctrl and mod:alt.'],
  ['stance:1', 'You are in that stance or form. 0 is caster form. Also form:1.'],
  ['stealth', 'You are stealthed.'],
  ['mounted', 'You are mounted.'],
  ['flying', 'You are flying.'],
  ['swimming', 'You are swimming.'],
  ['indoors', 'You are indoors.'],
  ['outdoors', 'You are outdoors.'],
  ['casting', 'You are casting something.'],
  ['channeling', 'You are channeling. channeling:Tranquility names the spell.'],
  ['known', 'You know the spell. known:Thorns names it.'],
  ['spec:1', 'That specialisation is active.'],
  ['group', 'You are in a group. group:raid and group:party narrow it.'],
  ['pet', 'You have a pet out. pet:Bear names it.'],
  ['button:1', 'Which mouse button pressed the key. 1 is left, 2 is right.'],
  ['equipped:Shields', 'You have that kind of item equipped.'],
  ['talent:1/1', 'That talent is chosen, by row and column.'],
]

const PHRASED = {
  nodead: 'The unit is alive. Pair it with help so a heal never lands on a corpse.',
  nomod: 'No modifier key is held.',
  nocombat: 'You are out of combat.',
}

const negated = ([text]) => {
  const key = `no${text}`
  return [key, PHRASED[key] || `The opposite of ${text}.`]
}

const CONDITIONS = [
  ...STATES,
  ...STATES.filter(([text]) => !text.startsWith('@')).map(negated),
]

const hintBox = () => $('m-hints')

let writing = null

let hints = { items: [], at: 0, start: 0, end: 0 }

const wordAt = (text, caret) => {
  const before = text.slice(0, caret)
  const open = before.lastIndexOf('[')
  const shut = before.lastIndexOf(']')
  const inside = open > shut
  const edge = inside
    ? Math.max(open, before.lastIndexOf(','), before.lastIndexOf(' '))
    : Math.max(before.lastIndexOf(' '), before.lastIndexOf('\n'))
  return { word: before.slice(edge + 1), start: edge + 1, inside }
}

const suggestionsFor = (word, inside) => {
  const table = inside ? CONDITIONS : SLASH
  const needle = word.toLowerCase()
  if (!inside && !word.startsWith('/') && !word.startsWith('#')) return []
  if (!needle) return inside ? table : []
  const opens = table.filter(([text]) => text.toLowerCase().startsWith(needle))
  const anywhere = () => table.filter(([text]) => text.toLowerCase().includes(needle))
  return opens.length || needle.length < 3 ? opens : anywhere()
}

const placeHints = () => {
  const box = hintBox()
  const spot = writing.getBoundingClientRect()
  box.style.left = `${spot.left}px`
  box.style.width = `${Math.max(spot.width, 420)}px`
  box.style.top = `${spot.bottom + 2}px`
}

const drawHints = () => {
  const box = hintBox()
  box.hidden = !hints.items.length
  box.replaceChildren(...hints.items.map(([text, about], i) => el('div', {
    className: i === hints.at ? 'hit on' : 'hit',
    onmousedown: (event) => {
      event.preventDefault()
      takeHint(i)
    },
  }, el('code', { textContent: text }), el('span', { textContent: about }))))
  if (hints.items.length) placeHints()
  const picked = box.children[hints.at]
  if (picked) picked.scrollIntoView({ block: 'nearest' })
}

const hideHints = () => {
  hints = { items: [], at: 0, start: 0, end: 0 }
  hintBox().hidden = true
}

const offerHints = () => {
  if (!writing) return hideHints()
  const { word, start, inside } = wordAt(writing.value, writing.selectionStart)
  hints = {
    items: suggestionsFor(word, inside).slice(0, 8),
    at: 0,
    start,
    end: writing.selectionStart,
  }
  drawHints()
}

const takeHint = (index) => {
  const [text] = hints.items[index] || []
  if (!text || !writing) return
  const box = writing
  box.value = box.value.slice(0, hints.start) + text + box.value.slice(hints.end)
  const caret = hints.start + text.length
  hideHints()
  box.focus()
  box.setSelectionRange(caret, caret)
  box.dispatchEvent(new Event('input', { bubbles: true }))
}

const MOVES = {
  ArrowDown: (step) => step + 1,
  ArrowUp: (step) => step - 1,
}

const onHintKey = (event) => {
  if (!hints.items.length) return
  const move = MOVES[event.key]
  if (move) {
    event.preventDefault()
    hints.at = (move(hints.at) + hints.items.length) % hints.items.length
    return drawHints()
  }
  if (event.key === 'Escape') return hideHints()
  if (event.key !== 'Enter' && event.key !== 'Tab') return
  event.preventDefault()
  takeHint(hints.at)
}

const copyMacro = (text) => navigator.clipboard.writeText(text)
  .then(() => toast('Copied. Paste it into the game with /macro.'))
  .catch(() => toast('Could not reach the clipboard'))

const grow = (box) => {
  box.style.height = 'auto'
  box.style.height = `${Math.max(box.scrollHeight, 34)}px`
}

const overLong = (draft) => [
  draft.name.length > 16 && `the game allows 16 characters in a name, this is ${draft.name.length}`,
  draft.body.length > 255 && `the game allows 255 characters in a macro, this is ${draft.body.length}`,
].find(Boolean)

const renderGameMacros = (list) => {
  const who = $('macro-who').selectedOptions[0]
  const name = who ? who.textContent.split(' · ')[0] : 'this character'
  $('macro-head').textContent = `${name}'s macros${list.length ? ` · ${list.length}` : ''}`
  $('macro-empty').hidden = list.length > 0
  $('macro-rows').replaceChildren(...list.map((m) => el('tr', {},
    el('td', {}, el('strong', { textContent: m.name })),
    el('td', {}, el('pre', { className: 'macro-body', textContent: m.body })),
    el('td', { className: 'row-actions' },
      el('button', { textContent: 'Copy', onclick: () => copyMacro(m.body) }),
      el('button', { textContent: 'Copy here', onclick: () => addDraft(m) }),
    ),
  )))
}

let fresh = null

const commitRow = (index, read, note) => {
  const draft = read()
  const tooLong = overLong(draft)
  note.textContent = tooLong || ''
  if (tooLong) return
  if (!draft.name.trim()) {
    note.textContent = index === null ? 'name it to keep it' : 'a macro needs a name'
    return
  }
  invoke('save_macro', { index, draft })
    .then(() => {
      fresh = null
      loadDrafts()
    })
    .catch((error) => (note.textContent = String(error)))
}

const draftRow = (m, index) => {
  const note = el('span', { className: 'cell-note' })
  const name = el('input', {
    className: 'cell-name', value: m.name, maxLength: 16, spellcheck: false, autocomplete: 'off',
    placeholder: 'name',
  })
  const body = el('textarea', {
    className: 'cell-body', value: m.body, spellcheck: false, rows: 1,
    placeholder: '/cast [@party1,help,nodead] Healing Touch',
  })
  const read = () => ({ name: name.value.trim(), body: body.value })
  const commit = () => commitRow(index, read, note)
  name.addEventListener('change', commit)
  body.addEventListener('change', commit)
  body.addEventListener('input', () => {
    grow(body)
    offerHints()
  })
  body.addEventListener('keydown', onHintKey)
  body.addEventListener('focus', () => {
    writing = body
    grow(body)
  })
  body.addEventListener('blur', hideHints)
  queueMicrotask(() => grow(body))
  return el('tr', {},
    el('td', {}, name),
    el('td', {}, body, note),
    el('td', { className: 'row-actions' },
      el('button', { textContent: 'Copy', onclick: () => copyMacro(body.value) }),
      index === null
        ? el('button', { textContent: 'Discard', onclick: () => { fresh = null; loadDrafts() } })
        : el('button', {
          textContent: 'Delete',
          onclick: () => ask('Delete this macro?', `"${m.name}" will go from your Warcry macros. The copy in the game is untouched.`)
            .then((yes) => yes && call('remove_macro', { index }).then(loadDrafts)),
        }),
    ),
  )
}

const renderDrafts = (list) => {
  const rows = list.map(draftRow)
  const pending = fresh ? [draftRow(fresh, null)] : []
  $('draft-head').textContent = `My macros${list.length ? ` · ${list.length}` : ''}`
  $('drafts-empty').hidden = list.length + pending.length > 0
  $('drafts').replaceChildren(...rows, ...pending)
  const last = pending.length && $('drafts').lastElementChild
  if (last) last.querySelector('.cell-name').focus()
}

const loadDrafts = () => call('macro_drafts').then(renderDrafts)

const addDraft = (m) => {
  fresh = { name: (m.name || '').slice(0, 16), body: m.body || '' }
  loadDrafts()
}

const whoLabel = (c) => [c.name, c.realm, `${c.flavor} \u00b7 ${c.account}`].filter(Boolean).join(' \u00b7 ')

const loadCharacters = () => call('wow_characters').then((found) => {
  const list = found || []
  const pick = $('macro-who')
  const keep = pick.value
  fill(pick, list.map((c) => [c.id, `${whoLabel(c)} \u2014 ${c.macros} macro${c.macros === 1 ? '' : 's'}`]), keep)
  if (!list.length) return renderGameMacros([])
  if (!list.some((c) => c.id === keep)) pick.value = list[0].id
  return loadGameMacros()
})

const loadGameMacros = () => {
  const id = $('macro-who').value
  return id ? call('wow_macros', { id }).then(renderGameMacros) : renderGameMacros([])
}

$('m-new').addEventListener('click', () => addDraft({}))
$('macro-who').addEventListener('change', loadGameMacros)
window.addEventListener('scroll', hideHints, true)
loadCharacters()
loadDrafts()

renderAddon()
loadTargets()
refresh()
setInterval(() => document.hidden || refresh(), 4000)
setInterval(() => document.hidden || loadTargets(), 4000)
