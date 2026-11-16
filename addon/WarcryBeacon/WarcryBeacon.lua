Warcry = Warcry or {}

local listeners = {}

function Warcry.on(event, fn)
  listeners[event] = listeners[event] or {}
  table.insert(listeners[event], fn)
end

function Warcry.emit(event, ...)
  for _, fn in ipairs(listeners[event] or {}) do
    local ok, why = pcall(fn, ...)
    if not ok then
      print("|cffffb347Warcry|r: " .. tostring(why))
    end
  end
end

local TONES = "Interface\\AddOns\\WarcryBeacon\\tones\\"
local SPACING = 0.23
local SETTLE = 1.0
local LURE_WARNING = 60

local CAST_STARTED = 0
local CAST_ENDED = 1
local CAUGHT = 2
local BAGS_FULL = 3
local LURE_LOW = 4
local LURE_GONE = 5
local ATTENTION = 6
local IDENTITY = 7

local FISHING = {
  [131474] = true,
  [131476] = true,
  [131490] = true,
  [7620] = true,
  [7731] = true,
  [7732] = true,
  [18248] = true,
  [33095] = true,
  [51294] = true,
  [88868] = true,
  [110410] = true,
  [158743] = true,
  [377895] = true,
}

local queue = {}
local sounding = false
local looted = false
local lure = {}

local function drain()
  local symbol = table.remove(queue, 1)
  if symbol == nil then
    sounding = false
    return
  end
  PlaySoundFile(TONES .. "t" .. symbol .. ".ogg", "Master")
  C_Timer.After(SPACING, drain)
end

local function emit(symbol)
  queue[#queue + 1] = symbol
  if sounding then
    return
  end
  sounding = true
  drain()
end

local IDENTITY_SPACE = 512

local function hash(name)
  local acc = 0
  for i = 1, #name do
    acc = (acc * 31 + string.byte(name, i)) % 4294967296
  end
  return acc % IDENTITY_SPACE
end

local function announce()
  local name = UnitName("player")
  if name == nil then
    return
  end
  local id = hash(string.lower(name))
  emit(IDENTITY)
  emit(math.floor(id / 64) % 8)
  emit(math.floor(id / 8) % 8)
  emit(id % 8)
end

local function checkLure()
  local has, expiry = GetWeaponEnchantInfo()
  if not has then
    if not lure.gone then
      lure.gone = true
      emit(LURE_GONE)
    end
    return
  end
  lure.gone = nil
  if expiry == nil then
    return
  end
  if expiry > LURE_WARNING * 1000 then
    lure.low = nil
    return
  end
  if not lure.low then
    lure.low = true
    emit(LURE_LOW)
  end
end

local function started(spellID)
  if not FISHING[spellID] then
    return
  end
  looted = false
  Warcry.casting = true
  Warcry.emit("cast")
  emit(CAST_STARTED)
  checkLure()
end

local function stopped(spellID)
  if not FISHING[spellID] then
    return
  end
  Warcry.casting = false
  Warcry.castEndedAt = GetTime()
  C_Timer.After(SETTLE, function()
    Warcry.emit("settled", looted)
    if looted then
      emit(CAUGHT)
      return
    end
    emit(CAST_ENDED)
  end)
end

local function warnBackground()
  if GetCVar("Sound_EnableSoundWhenGameIsInBG") ~= "0" then
    return
  end
  print("|cffffb347Warcry|r: turn on Sound in Background (Options, Sound) so this window still beeps when it is not focused.")
end

local handlers = {}

function handlers.PLAYER_LOGIN()
  warnBackground()
  announce()
  Warcry.emit("login")
end

function handlers.PLAYER_ENTERING_WORLD(initial, reloading)
  if not initial and not reloading then
    return
  end
  announce()
end

function handlers.UNIT_SPELLCAST_CHANNEL_START(_, _, spellID)
  started(spellID)
end

function handlers.UNIT_SPELLCAST_CHANNEL_STOP(_, _, spellID)
  stopped(spellID)
end

function handlers.LOOT_OPENED()
  looted = true
end

function handlers.CHAT_MSG_LOOT(message)
  if string.find(message or "", "|Hitem:") then
    looted = true
  end
end

function handlers.UI_ERROR_MESSAGE(_, message)
  if message ~= ERR_INV_FULL then
    return
  end
  emit(BAGS_FULL)
end

function handlers.CHAT_MSG_WHISPER()
  emit(ATTENTION)
end

function handlers.PLAYER_DEAD()
  emit(ATTENTION)
end

local frame = CreateFrame("Frame")
frame:SetScript("OnEvent", function(_, event, ...)
  local handler = handlers[event]
  if handler == nil then
    return
  end
  handler(...)
end)

for event in pairs(handlers) do
  if event ~= "UNIT_SPELLCAST_CHANNEL_START" and event ~= "UNIT_SPELLCAST_CHANNEL_STOP" then
    frame:RegisterEvent(event)
  end
end
frame:RegisterUnitEvent("UNIT_SPELLCAST_CHANNEL_START", "player")
frame:RegisterUnitEvent("UNIT_SPELLCAST_CHANNEL_STOP", "player")

local function help()
  print("|cffffb347Warcry|r: /warcry gold, /warcry chat, /warcry reset, /warcry id")
end

SLASH_WARCRY1 = "/warcry"
SlashCmdList.WARCRY = function(argument)
  local word = string.lower(string.trim(argument or ""))
  if word == "gold" or word == "fish" then
    Warcry.goldUI.toggle()
    return
  end
  if word == "chat" then
    local on = Warcry.ledger.chatty()
    print("|cffffb347Warcry|r: catch messages in chat " .. (on and "on" or "off"))
    return
  end
  if word == "quiet" or word == "mute" then
    print("|cffffb347Warcry|r: the beeps are how Warcry follows the game, so the addon cannot stop them. Mute World of Warcraft in the Warcry app instead (Phone tab), which silences the game to your speakers while Warcry still hears it.")
    return
  end
  if word == "reset" then
    Warcry.ledger.start()
    Warcry.goldUI.show()
    print("|cffffb347Warcry|r: fishing session reset")
    return
  end
  if word == "id" or word == "" then
    print("|cffffb347Warcry Beacon|r: sounding identity")
    announce()
    return
  end
  help()
end
