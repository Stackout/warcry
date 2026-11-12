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
  off: 'Stopped',
  on: 'Listening',
  paused: 'Paused',
}

const renderTarget = (target) => {
  const box = $('target')
  box.replaceChildren('Whatever window is focused')
  if (!target) return
  const found = target.found
    ? el('span', { className: 'found', textContent: `✓ ${target.found}` })
    : el('span', { className: 'missing', textContent: 'not open right now' })
  box.replaceChildren(el('b', { textContent: target.description }), el('br'), found)
}

let editing = null

const draftFields = {
  name: 'd-name',
  phrase: 'd-phrase',
  keys: 'd-keys',
  hotkey: 'd-hotkey',
  window: 'd-window',
  mode: 'd-mode',
}

const showEditor = (index, draft) => {
  editing = index
  Object.entries(draftFields).forEach(([key, id]) => ($(id).value = draft[key] || ''))
  $('d-mode').value = draft.mode || 'background'
  $('d-active').checked = !!draft.always_active
  $('d-error').textContent = ''
  $('editor').hidden = false
  $('add-command').hidden = true
  $('d-name').focus()
  call('target_windows').then((titles) =>
    $('window-list').replaceChildren(...(titles || []).map((t) => el('option', { value: t })))
  )
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
  invoke('edit_command', { index: editing, draft })
    .then(hideEditor)
    .then(refresh)
    .catch((error) => ($('d-error').textContent = String(error)))
}

const removeCommand = (c) => {
  if (!confirm(`Delete “${c.name}”?`)) return
  call('remove_command', { index: c.index }).then(refresh)
}

const commandRow = (c) => el('li', {},
  el('header', {}, el('h4', { textContent: c.name }), c.hotkey ? el('kbd', { textContent: c.hotkey }) : ''),
  el('div', { className: 'phrases' }, ...c.phrases.map((p) => el('span', { textContent: `“${p}”` })), c.more ? el('span', { textContent: `+${c.more} more` }) : ''),
  el('div', { className: 'meta', textContent: `${c.actions} action${c.actions === 1 ? '' : 's'}${c.always_active ? ' · always active' : ''}${c.draft.window ? ` · ${c.draft.window}` : ''}` }),
  el('div', { className: 'editor-row' },
    c.editable
      ? el('button', { textContent: 'Edit', onclick: () => showEditor(c.index, c.draft) })
      : el('span', { className: 'meta', textContent: 'too detailed to edit here, use Edit profile' }),
    el('button', { textContent: 'Delete', onclick: () => removeCommand(c) }),
  ),
)

const renderCommands = (commands) => {
  $('command-count').textContent = commands.length || ''
  $('commands').replaceChildren(...commands.map(commandRow))
}

const render = (s) => {
  state = s
  if (s.failure) toast(s.failure)
  const mode = [s.busy && 'busy', !s.running && 'off', s.listening && 'on', 'paused'].find(Boolean)
  $('status').className = `pill ${mode}`
  $('status').querySelector('b').textContent = STATUS[mode]
  $('ptt').hidden = !s.push_to_talk
  $('ptt').textContent = `Hold ${s.push_to_talk} to talk`
  const power = $('power')
  power.querySelector('span').textContent = { true: 'Stop', false: 'Start' }[s.running]
  power.classList.toggle('stop', s.running)
  power.disabled = s.busy || !s.profile
  $('listening').checked = s.listening
  $('listening').disabled = !s.running
  $('profile-name').textContent = s.profile_name || (s.profile ? basename(s.profile) : 'No profile yet')
  fill($('profile'), s.profiles.map((p) => [p, basename(p)]), s.profile)
  $('profile').hidden = !s.profiles.length
  $('edit').disabled = !s.profile
  $('profile-error').hidden = !s.profile_error
  $('profile-error').textContent = s.profile_error || ''
  renderTarget(s.target)
  fill($('device'), [['', 'System default'], ...s.devices.map((d) => [d, d])], s.config.device || '')
  fill($('model'), s.models.map((m) => [m.name, `${m.name}${m.downloaded ? '' : ' ↓'} · ${m.about}`]), s.model)
  const current = s.models.find((m) => m.name === s.model)
  $('download').hidden = !current || current.downloaded || !$('progress').hidden
  $('auto-start').checked = s.config.auto_start
  $('welcome').hidden = Boolean(s.profile)
  $('feed-empty').hidden = !s.profile || $('feed').children.length > 0
  renderCommands(s.commands)
}

const refresh = () => call('snapshot').then(render)

const saveConfig = (patch) => call('set_config', { config: { ...state.config, ...patch } }).then(refresh)

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
  $('download').hidden = true
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
listen('log', ({ payload }) => appendLog(payload))
listen('changed', refresh)
listen('failure', refresh)

$('power').addEventListener('click', () => {
  const command = { true: 'stop', false: 'start' }[state.running]
  render({ ...state, busy: !state.running })
  call(command).finally(refresh)
})
$('listening').addEventListener('change', (e) => call('set_listening', { on: e.target.checked }).then(refresh))
$('profile').addEventListener('change', (e) => saveConfig({ profile: e.target.value }))
$('device').addEventListener('change', (e) => saveConfig({ device: e.target.value || null }))
$('model').addEventListener('change', (e) => saveConfig({ model: e.target.value }))
$('auto-start').addEventListener('change', (e) => saveConfig({ auto_start: e.target.checked }))
$('download').addEventListener('click', () => call('download_model', { name: state.model }).finally(refresh))
$('edit').addEventListener('click', () => call('open_profile'))
$('add-command').addEventListener('click', () => showEditor(null, { mode: 'background' }))
$('d-save').addEventListener('click', saveDraft)
$('d-cancel').addEventListener('click', hideEditor)
$('folder').addEventListener('click', () => call('open_folder'))
const create = (example) => call('create_profile', { example }).then(refresh)
$('new-fishing').addEventListener('click', () => create('fishing'))
$('new-starter').addEventListener('click', () => create('starter'))
$('welcome-fishing').addEventListener('click', () => create('fishing'))

$('try').addEventListener('submit', (e) => {
  e.preventDefault()
  const text = $('try-text').value.trim()
  if (!text) return
  call('try_phrase', { text }).then((result) => { $('try-result').textContent = result })
})

document.querySelectorAll('.tabs button').forEach((button) => button.addEventListener('click', () => {
  document.querySelectorAll('.tabs button').forEach((b) => b.classList.toggle('active', b === button))
  document.querySelectorAll('.tab').forEach((tab) => { tab.hidden = tab.id !== `tab-${button.dataset.tab}` })
}))

refresh()
setInterval(() => document.hidden || refresh(), 4000)
