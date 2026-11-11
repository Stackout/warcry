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
  mode: 'd-mode',
}

const windowOption = (choice) => el('option', {
  value: String(choice.pid),
  textContent: `${choice.title || choice.process} — ${choice.process}, pid ${choice.pid}`,
  dataset: undefined,
})

const fillWindows = (draft) => call('target_windows').then((choices) => {
  const pick = $('d-window-pick')
  const live = choices || []
  const options = [el('option', { value: '', textContent: 'whatever is focused' })]
  live.forEach((choice) => {
    const option = windowOption(choice)
    option.setAttribute('data-title', choice.title)
    option.setAttribute('data-process', choice.process)
    options.push(option)
  })
  const saved = draft.pid || ''
  if (saved && !live.some((c) => String(c.pid) === String(saved))) {
    const gone = el('option', {
      value: String(saved),
      textContent: `${draft.window || 'saved window'} — not running`,
    })
    gone.setAttribute('data-title', draft.window || '')
    gone.setAttribute('data-process', draft.process || '')
    options.push(gone)
  }
  pick.replaceChildren(...options)
  pick.value = saved
})

const showEditor = (index, draft) => {
  editing = index
  Object.entries(draftFields).forEach(([key, id]) => ($(id).value = draft[key] || ''))
  $('d-mode').value = draft.mode || 'background'
  $('d-active').checked = !!draft.always_active
  $('d-error').textContent = ''
  $('editor').hidden = false
  $('add-command').hidden = true
  $('d-name').focus()
  fillWindows(draft)
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
  const picked = $('d-window-pick').selectedOptions[0]
  draft.pid = $('d-window-pick').value
  draft.window = picked ? picked.getAttribute('data-title') || '' : ''
  draft.process = picked ? picked.getAttribute('data-process') || '' : ''
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

const commandRow = (c) => el('li', {},
  el('header', {}, el('h4', { textContent: c.name }), c.hotkey ? el('kbd', { textContent: c.hotkey }) : ''),
  el('div', { className: 'phrases' }, ...c.phrases.map((p) => el('span', { textContent: `“${p}”` })), c.more ? el('span', { textContent: `+${c.more} more` }) : ''),
  el('div', { className: 'meta', textContent: `${c.actions} action${c.actions === 1 ? '' : 's'}${c.always_active ? ' · always active' : ''}${c.draft.window || c.draft.process ? ` · ${[c.draft.window, c.draft.process].filter(Boolean).join(' / ')}` : ''}` }),
  el('div', { className: 'editor-row' },
    c.editable
      ? el('button', { textContent: 'Edit', onclick: () => showEditor(c.index, c.draft) })
      : el('span', { className: 'meta', textContent: 'too detailed to edit here, use Edit profile' }),
    el('button', {
      textContent: 'Delete',
      onclick: (event) => removeCommand(event.currentTarget, c),
    }),
  ),
)

const renderCommands = (commands) => {
  $('command-count').textContent = commands.length || ''
  $('commands').replaceChildren(...commands.map(commandRow))
}

const flavorName = (label) => {
  const tail = label.split(' ').pop() || label
  const bare = tail.replace(/^_|_$/g, '').replace(/_/g, ' ')
  return bare.charAt(0).toUpperCase() + bare.slice(1)
}

const renderAddon = () => call('wow_addon').then((installs) => {
  const box = $('addon')
  const list = installs || []
  $('addon-version').textContent = list.length ? `v${list[0].bundled}` : ''
  if (!list.length) {
    return box.replaceChildren(el('p', { className: 'meta', textContent: 'No World of Warcraft found' }))
  }
  const stale = list.filter((a) => a.installed !== a.bundled)
  box.replaceChildren(
    ...list.map((a) => el('div', { className: 'addon-row' },
      el('span', { textContent: flavorName(a.label) }),
      el('span', {
        className: a.installed === a.bundled ? 'found' : 'meta',
        textContent: a.installed === a.bundled ? 'up to date' : (a.installed ? `v${a.installed}` : 'not installed'),
      }),
    )),
    el('button', {
      className: stale.length ? 'primary' : 'ghost small',
      textContent: stale.length ? `Install into ${stale.length} ${stale.length === 1 ? 'client' : 'clients'}` : 'Reinstall',
      onclick: (event) => {
        event.currentTarget.disabled = true
        const wanted = stale.length ? stale : list
        Promise.all(wanted.map((a) => invoke('install_addon', { flavor: a.flavor })))
          .then(() => toast('Addon installed. Restart World of Warcraft.'))
          .catch((error) => toast(String(error)))
          .finally(renderAddon)
      },
    }),
  )
})

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
renderAddon()
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
