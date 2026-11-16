local IDLE = 120
local GRACE = 5
local TOP = 5

local ledger = {}
local ui = {}
local told

local function itemInfo(link)
  local getter = C_Item and C_Item.GetItemInfo or GetItemInfo
  return getter(link)
end

local function sellPrice(link)
  local price = select(11, itemInfo(link))
  return price or 0
end

local function auctionator()
  local api = Auctionator and Auctionator.API and Auctionator.API.v1
  if api == nil or api.GetAuctionPriceByItemLink == nil then
    return nil
  end
  return api
end

local function asked(call, link)
  if call == nil then
    return nil
  end
  local ok, price = pcall(call, "WarcryBeacon", link)
  if not ok then
    return nil
  end
  return price
end

local function priced(link)
  local api = auctionator()
  local auction = api and asked(api.GetAuctionPriceByItemLink, link) or nil
  local vendor = api and asked(api.GetVendorPriceByItemLink, link) or nil
  return auction or 0, vendor or sellPrice(link)
end

local function blank()
  return {
    started = time(),
    seconds = 0,
    casts = 0,
    catches = 0,
    auction = 0,
    vendor = 0,
    fish = {},
    zones = {},
  }
end

local function store()
  WarcryLedgerDB = WarcryLedgerDB or { version = 1, sessions = {} }
  WarcryLedgerDB.sessions = WarcryLedgerDB.sessions or {}
  return WarcryLedgerDB
end

local session = nil
local lastTouch = nil
local castEnded = 0

local function touch()
  local now = GetTime()
  if lastTouch ~= nil and now - lastTouch < IDLE then
    session.seconds = session.seconds + (now - lastTouch)
  end
  lastTouch = now
end

local function zone()
  local sub = GetSubZoneText()
  if sub == nil or sub == "" then
    return GetZoneText() or "somewhere"
  end
  return sub
end

local function bucket(map, key, maker)
  if map[key] == nil then
    map[key] = maker()
  end
  return map[key]
end

function ledger.start()
  session = blank()
  lastTouch = nil
  castEnded = 0
  table.insert(store().sessions, session)
end

function ledger.cast()
  if session == nil then
    ledger.start()
  end
  touch()
  session.casts = session.casts + 1
end

function ledger.settled(caught)
  if session == nil then
    ledger.start()
  end
  castEnded = GetTime()
  if caught then
    return
  end
  ui.refresh()
end

local function open()
  local ended = math.max(Warcry.castEndedAt or 0, castEnded)
  return ended > 0 and GetTime() - ended < GRACE
end

function ledger.loot(link, count)
  if session == nil or not (Warcry.casting or open()) then
    return
  end
  local name = itemInfo(link) or link
  local id = tonumber(string.match(link, "item:(%d+)")) or 0
  local auction, vendor = priced(link)
  touch()
  session.catches = session.catches + count
  session.auction = session.auction + auction * count
  session.vendor = session.vendor + vendor * count
  local fish = bucket(session.fish, tostring(id), function()
    return { name = name, count = 0, auction = 0, vendor = 0 }
  end)
  fish.name = name
  fish.count = fish.count + count
  fish.auction = fish.auction + auction * count
  fish.vendor = fish.vendor + vendor * count
  local here = bucket(session.zones, zone(), function()
    return { catches = 0, auction = 0, seconds = 0 }
  end)
  here.catches = here.catches + count
  here.auction = here.auction + auction * count
  here.seconds = session.seconds
  told(name, count, auction * count, vendor * count)
  ui.refresh()
end

function ledger.chatty()
  local db = store()
  db.quiet = not db.quiet
  return not db.quiet
end

function ledger.perHour(copper, seconds)
  if seconds < 60 then
    return 0
  end
  return math.floor(copper / (seconds / 3600))
end

function ledger.best(fish, limit)
  local rows = {}
  for _, entry in pairs(fish) do
    table.insert(rows, entry)
  end
  table.sort(rows, function(a, b)
    if a.auction == b.auction then
      return a.count > b.count
    end
    return a.auction > b.auction
  end)
  while #rows > limit do
    table.remove(rows)
  end
  return rows
end

function ledger.summary()
  if session == nil then
    ledger.start()
  end
  local worth = math.max(session.auction, session.vendor)
  return {
    gold = worth,
    rate = ledger.perHour(worth, session.seconds),
    casts = session.casts,
    catches = session.catches,
    missed = math.max(session.casts - session.catches, 0),
    seconds = session.seconds,
    auction = session.auction,
    vendor = session.vendor,
    zone = zone(),
    best = ledger.best(session.fish, TOP),
  }
end

function told(name, count, auction, vendor)
  if store().quiet then
    return
  end
  local worth = math.max(auction, vendor)
  print(string.format(
    "|cffffb347Warcry|r: %s x%d  %s  (%d this session)",
    name, count, GetCoinTextureString(worth), session.catches
  ))
end

local function money(copper)
  if copper <= 0 then
    return "0g"
  end
  return GetCoinTextureString(copper)
end

local function percent(part, whole)
  if whole <= 0 then
    return "0%"
  end
  return string.format("%d%%", math.floor(part / whole * 100 + 0.5))
end

local function clock(seconds)
  return string.format("%d:%02d", math.floor(seconds / 60), math.floor(seconds % 60))
end

local frame = nil

local function line(parent, previous, size, colour)
  local text = parent:CreateFontString(nil, "OVERLAY", "GameFontNormal")
  text:SetFont(STANDARD_TEXT_FONT, size, "")
  text:SetTextColor(unpack(colour))
  text:SetJustifyH("LEFT")
  if previous == nil then
    text:SetPoint("TOPLEFT", parent, "TOPLEFT", 12, -30)
    return text
  end
  text:SetPoint("TOPLEFT", previous, "BOTTOMLEFT", 0, -4)
  return text
end

local function build()
  local f = CreateFrame("Frame", "WarcryLedgerFrame", UIParent)
  f:SetSize(240, 190)
  f:SetPoint("CENTER", UIParent, "CENTER", 320, 120)
  f:SetMovable(true)
  f:EnableMouse(true)
  f:RegisterForDrag("LeftButton")
  f:SetScript("OnDragStart", f.StartMoving)
  f:SetScript("OnDragStop", f.StopMovingOrSizing)
  f:SetClampedToScreen(true)
  local back = f:CreateTexture(nil, "BACKGROUND")
  back:SetAllPoints(f)
  back:SetColorTexture(0, 0, 0, 0.72)
  local title = f:CreateFontString(nil, "OVERLAY", "GameFontNormal")
  title:SetFont(STANDARD_TEXT_FONT, 13, "")
  title:SetPoint("TOPLEFT", f, "TOPLEFT", 12, -10)
  title:SetText("|cffffb347Warcry|r fishing")
  local close = CreateFrame("Button", nil, f, "UIPanelCloseButton")
  close:SetPoint("TOPRIGHT", f, "TOPRIGHT", -2, -2)
  close:SetScript("OnClick", function() f:Hide() end)
  f.gold = line(f, nil, 16, { 1, 0.82, 0 })
  f.rate = line(f, f.gold, 12, { 0.75, 0.9, 0.75 })
  f.casts = line(f, f.rate, 11, { 0.8, 0.8, 0.8 })
  f.advice = line(f, f.casts, 11, { 0.9, 0.7, 0.4 })
  f.here = line(f, f.advice, 11, { 0.7, 0.8, 0.95 })
  f.top = line(f, f.here, 11, { 0.85, 0.85, 0.85 })
  f.top:SetWidth(216)
  f.top:SetJustifyV("TOP")
  return f
end

function ui.refresh()
  if frame == nil or not frame:IsShown() then
    return
  end
  local s = ledger.summary()
  frame.gold:SetText(money(s.gold))
  frame.rate:SetText(money(s.rate) .. " /hr  ·  " .. clock(s.seconds) .. " fishing")
  frame.casts:SetText(string.format(
    "%d casts · %d caught · %d missed (%s)",
    s.casts, s.catches, s.missed, percent(s.missed, s.casts)
  ))
  if s.vendor > s.auction then
    frame.advice:SetText("vendor beats the AH here: " .. money(s.vendor))
  else
    frame.advice:SetText("AH " .. money(s.auction) .. " · vendor " .. money(s.vendor))
  end
  local here = session.zones[s.zone]
  if here == nil then
    frame.here:SetText(s.zone .. ": nothing yet")
  else
    frame.here:SetText(string.format("%s: %s from %d", s.zone, money(here.auction), here.catches))
  end
  local rows = {}
  for _, fish in ipairs(s.best) do
    table.insert(rows, string.format("%s x%d  %s", fish.name, fish.count, money(fish.auction)))
  end
  frame.top:SetText(table.concat(rows, "\n"))
end

function ui.toggle()
  if frame == nil then
    frame = build()
  end
  if frame:IsShown() then
    frame:Hide()
    return
  end
  frame:Show()
  ui.refresh()
end

function ui.show()
  if frame == nil then
    frame = build()
  end
  frame:Show()
  ui.refresh()
end

local function lootLink(message)
  return string.match(message, "(|c%x+|Hitem:.-|h.-|h|r)") or string.match(message, "(|Hitem:.-|h.-|h)")
end

local function lootCount(message)
  return tonumber(string.match(message, "|r?x(%d+)")) or tonumber(string.match(message, "x(%d+)%.?$")) or 1
end

local events = CreateFrame("Frame")
events:RegisterEvent("CHAT_MSG_LOOT")
events:SetScript("OnEvent", function(_, _, message)
  local link = lootLink(message)
  if link == nil then
    return
  end
  ledger.loot(link, lootCount(message))
end)

Warcry.ledger = ledger
Warcry.goldUI = ui
Warcry.on("login", ledger.start)
Warcry.on("cast", ledger.cast)
Warcry.on("settled", ledger.settled)
