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
}

const capLabel = (part) => CAP_LABELS[part] || CAP_GLYPHS[part]
  || (part.length === 1 ? part.toUpperCase() : part)

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

const armCapture = (id, hint) => {
  const input = $(id)
  if (!input) return
  input.hidden = true
  const slot = el('span', { className: 'slot' })
  const field = el('button', { type: 'button', className: 'keyfield' }, slot)
  const wipe = el('button', { type: 'button', className: 'keyclear', textContent: '\u00d7', title: 'clear' })
  const paint = () => {
    field.classList.remove('arming')
    slot.replaceChildren(input.value ? keycaps(input.value) : el('em', { textContent: hint }))
    wipe.hidden = !input.value
  }
  const set = (value) => {
    input.value = value
    paint()
  }
  const onKey = (event) => {
    event.preventDefault()
    event.stopPropagation()
    document.removeEventListener('keydown', onKey, true)
    set((event.code === 'Escape' ? null : comboOf(event)) || input.value)
  }
  field.addEventListener('click', () => {
    field.classList.add('arming')
    slot.replaceChildren(el('em', { textContent: 'press any key\u2026' }))
    document.addEventListener('keydown', onKey, true)
  })
  wipe.addEventListener('click', () => set(''))
  input.after(el('span', { className: 'keyrow' }, field, wipe))
  repaints.push(paint)
  paint()
}

const showEditor = (index, draft) => {
  editing = index
  Object.entries(draftFields).forEach(([key, id]) => ($(id).value = draft[key] || ''))
  $('d-active').checked = !!draft.always_active
  $('d-pad').checked = !!draft.pad
  repaints.forEach((paint) => paint())
  $('d-error').textContent = ''
  $('editor').hidden = false
  $('add-command').hidden = true
  $('d-name').focus()
}

const hideEditor = () => {
  editing = null
  $('editor').hidden = true
  $('add-command').hidden = false
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

const removeCommand = (button, c) => {
  if (button.dataset.sure !== 'yes') {
    button.dataset.sure = 'yes'
    button.textContent = 'Really delete?'
    setTimeout(() => {
      button.dataset.sure = ''
      button.textContent = 'Delete'
    }, 4000)
    return
  }
  call('remove_command', { index: c.index }).then(refresh)
}

const haystack = (c) => [c.name, c.draft.keys, c.hotkey, ...c.phrases]
  .filter(Boolean)
  .join(' ')
  .toLowerCase()

const phraseCell = (c) => el('td', { className: 'say' },
  ...c.phrases.slice(0, 2).map((phrase) => el('span', { textContent: `“${phrase}”` })),
  c.more || c.phrases.length > 2
    ? el('span', { className: 'dim', textContent: `+${c.more + Math.max(0, c.phrases.length - 2)} more` })
    : '')

const commandRow = (c) => el('tr', { hay: haystack(c) },
  el('td', {}, el('strong', { textContent: c.name }),
    c.actions > 1 ? el('span', { className: 'dim', textContent: ` ${c.actions} actions` }) : ''),
  phraseCell(c),
  el('td', {}, c.draft.keys ? keycaps(c.draft.keys, 'the key this sends into the game') : el('span', { className: 'dim', textContent: '—' })),
  el('td', {}, c.hotkey ? keycaps(c.hotkey, 'global hotkey', 'hot') : el('span', { className: 'dim', textContent: '—' })),
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
    el('button', {
      textContent: 'Delete',
      onclick: (event) => removeCommand(event.currentTarget, c),
    }),
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
$('p-delete').addEventListener('click', (event) => {
  const button = event.currentTarget
  if (button.dataset.sure !== 'yes') {
    button.dataset.sure = 'yes'
    button.textContent = 'Really delete it?'
    setTimeout(() => {
      button.dataset.sure = ''
      button.textContent = 'Delete this profile'
    }, 4000)
    return
  }
  button.dataset.sure = ''
  button.textContent = 'Delete this profile'
  profileCall('delete_profile')
})
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

const CONDITIONS = [
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
  ['nodead', 'The unit is alive. Pair it with help so a heal never lands on a corpse.'],
  ['noexists', 'The unit is not there.'],
  ['nohelp', 'The unit is not friendly.'],
  ['noharm', 'The unit is not attackable.'],
  ['combat', 'You are in combat.'],
  ['nocombat', 'You are out of combat.'],
  ['mod', 'Any modifier key is held.'],
  ['mod:shift', 'Shift is held. Also mod:ctrl and mod:alt.'],
  ['nomod', 'No modifier key is held.'],
  ['stance:1', 'You are in that stance or form. 0 is caster form. Also form:1.'],
  ['stealth', 'You are stealthed.'],
  ['nostealth', 'You are not stealthed.'],
  ['mounted', 'You are mounted.'],
  ['nomounted', 'You are not mounted.'],
  ['flying', 'You are flying.'],
  ['noflying', 'You are not flying.'],
  ['swimming', 'You are swimming.'],
  ['indoors', 'You are indoors.'],
  ['outdoors', 'You are outdoors.'],
  ['casting', 'You are casting something.'],
  ['nocasting', 'You are not casting.'],
  ['channeling', 'You are channeling. channeling:Tranquility names the spell.'],
  ['nochanneling', 'You are not channeling. nochanneling:Drain Life names the spell.'],
  ['known', 'You know the spell. known:Thorns names it.'],
  ['spec:1', 'That specialisation is active.'],
  ['group', 'You are in a group. group:raid and group:party narrow it.'],
  ['nogroup', 'You are on your own.'],
  ['pet', 'You have a pet out. pet:Bear names it.'],
  ['nopet', 'You have no pet out.'],
  ['button:1', 'Which mouse button pressed the key. 1 is left, 2 is right.'],
  ['equipped:Shields', 'You have that kind of item equipped.'],
  ['talent:1/1', 'That talent is chosen, by row and column.'],
]

const hintBox = () => $('m-hints')

let hints = { items: [], at: 0, start: 0, end: 0 }

const wordAt = (text, caret) => {
  const before = text.slice(0, caret)
  const open = before.lastIndexOf('[')
  const shut = before.lastIndexOf(']')
  const inside = open > shut
  const edge = inside ? Math.max(open, before.lastIndexOf(','), before.lastIndexOf(' ')) : Math.max(before.lastIndexOf(' '), before.lastIndexOf('\n'))
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

const drawHints = () => {
  const box = hintBox()
  box.hidden = !hints.items.length
  box.replaceChildren(...hints.items.map(([text, about], i) => el('div', {
    className: i === hints.at ? 'hit on' : 'hit',
    onmousedown: (event) => { event.preventDefault(); takeHint(i) },
  }, el('code', { textContent: text }), el('span', { textContent: about }))))
  const picked = box.children[hints.at]
  if (picked) picked.scrollIntoView({ block: 'nearest' })
}

const offerHints = () => {
  const body = $('m-body')
  const { word, start, inside } = wordAt(body.value, body.selectionStart)
  hints = { items: suggestionsFor(word, inside).slice(0, 8), at: 0, start, end: body.selectionStart }
  drawHints()
}

const takeHint = (index) => {
  const body = $('m-body')
  const [text] = hints.items[index] || []
  if (!text) return
  body.value = body.value.slice(0, hints.start) + text + body.value.slice(hints.end)
  const caret = hints.start + text.length
  body.setSelectionRange(caret, caret)
  hideHints()
  countBody()
  body.focus()
}

const hideHints = () => {
  hints = { items: [], at: 0, start: 0, end: 0 }
  hintBox().hidden = true
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

const countBody = () => {
  const used = $('m-body').value.length
  $('m-count').textContent = `${used} / 255`
  $('m-count').className = used > 255 ? 'missing' : 'meta'
}

let draftAt = null

const copyMacro = (button, text) => navigator.clipboard.writeText(text)
  .then(() => toast('Copied. Paste it into the game with /macro.'))
  .catch(() => toast('Could not reach the clipboard'))

const macroRow = (m, actions) => el('tr', {},
  el('td', {}, el('strong', { textContent: m.name })),
  el('td', {}, el('pre', { className: 'macro-body', textContent: m.body })),
  el('td', { className: 'row-actions' }, ...actions),
)

const renderGameMacros = (list) => {
  $('macro-tally').textContent = list.length ? `${list.length}` : ''
  $('macro-empty').hidden = list.length > 0
  $('macro-rows').replaceChildren(...list.map((m) => macroRow(m, [
    el('button', { textContent: 'Copy', onclick: (e) => copyMacro(e.currentTarget, m.body) }),
    el('button', { textContent: 'Edit here', onclick: () => editDraft(null, m) }),
  ])))
}

const editDraft = (index, draft) => {
  draftAt = index
  $('m-name').value = (draft.name || '').slice(0, 16)
  $('m-body').value = draft.body || ''
  $('m-error').textContent = ''
  countBody()
  $('m-name').focus()
}

const renderDrafts = (list) => {
  $('draft-tally').textContent = list.length ? `${list.length}` : ''
  $('drafts-empty').hidden = list.length > 0
  $('drafts').replaceChildren(...list.map((m, index) => macroRow(m, [
    el('button', { textContent: 'Copy', onclick: (e) => copyMacro(e.currentTarget, m.body) }),
    el('button', { textContent: 'Edit', onclick: () => editDraft(index, m) }),
    el('button', { textContent: 'Delete', onclick: () => call('remove_macro', { index }).then(loadDrafts) }),
  ])))
}

const loadDrafts = () => call('macro_drafts').then(renderDrafts)

const whoLabel = (c) => [c.name, c.realm, `${c.flavor} · ${c.account}`].filter(Boolean).join(' · ')

const loadCharacters = () => call('wow_characters').then((found) => {
  const list = found || []
  const pick = $('macro-who')
  const keep = pick.value
  fill(pick, list.map((c) => [c.id, `${whoLabel(c)} — ${c.macros} macro${c.macros === 1 ? '' : 's'}`]), keep)
  if (!list.length) return renderGameMacros([])
  if (!list.some((c) => c.id === keep)) pick.value = list[0].id
  return loadGameMacros()
})

const loadGameMacros = () => {
  const id = $('macro-who').value
  return id ? call('wow_macros', { id }).then(renderGameMacros) : renderGameMacros([])
}

const saveMacro = () => {
  const draft = { name: $('m-name').value.trim(), body: $('m-body').value }
  invoke('save_macro', { index: draftAt, draft })
    .then(() => { draftAt = null; $('m-name').value = ''; $('m-body').value = ''; countBody() })
    .then(loadDrafts)
    .catch((error) => ($('m-error').textContent = String(error)))
}

$('m-body').addEventListener('input', () => { countBody(); offerHints() })
$('m-body').addEventListener('keydown', onHintKey)
$('m-body').addEventListener('blur', hideHints)
$('m-save').addEventListener('click', saveMacro)
$('m-new').addEventListener('click', () => editDraft(null, {}))
$('macro-who').addEventListener('change', loadGameMacros)
loadCharacters()
loadDrafts()
countBody()

renderAddon()
loadTargets()
refresh()
setInterval(() => document.hidden || refresh(), 4000)
setInterval(() => document.hidden || loadTargets(), 4000)
