local here = arg[0]:match("(.*/)") or "./"
package.path = here .. "?.lua;" .. package.path
local wow = require("wow")

local failures = 0
local checks = 0

local function check(name, got, want)
  checks = checks + 1
  if got ~= want then
    failures = failures + 1
    print(string.format("FAIL %s\n  got  %s\n  want %s", name, tostring(got), tostring(want)))
    return
  end
  print("ok   " .. name)
end

local function load()
  wow.reset()
  local chunk = assert(loadfile(here .. "../WarcryBeacon/WarcryLedger.lua"))
  chunk()
  Warcry.ledger.start()
  return Warcry.ledger
end

local function fish(ledger, id, name, count)
  Warcry.casting = true
  ledger.cast()
  wow.now = wow.now + 20
  ledger.loot(wow.link(id, name), count or 1)
  Warcry.casting = false
  ledger.settled(true)
end

do
  local ledger = load()
  Warcry.casting = true
  ledger.cast()
  Warcry.casting = false
  ledger.settled(false)
  local s = ledger.summary()
  check("a cast with no loot is a miss", s.casts .. "/" .. s.catches .. "/" .. s.missed, "1/0/1")
end

do
  local ledger = load()
  wow.prices[13756] = 5000
  fish(ledger, 13756, "Raw Summer Bass")
  local s = ledger.summary()
  check("a caught fish is worth its auction price", s.gold, 5000)
  check("a caught fish is not a miss", s.missed, 0)
  check("the catch is counted", s.catches, 1)
end

do
  local ledger = load()
  wow.prices[13756] = 5000
  fish(ledger, 13756, "Raw Summer Bass", 3)
  check("a stack counts every fish", ledger.summary().gold, 15000)
  check("a stack counts every catch", ledger.summary().catches, 3)
end

do
  local ledger = load()
  wow.prices[6303] = 100
  wow.sell[6303] = 900
  fish(ledger, 6303, "Raw Glossy Mightfish")
  local s = ledger.summary()
  check("vendor wins when it beats the auction house", s.gold, 900)
  check("both prices are kept", s.auction .. "/" .. s.vendor, "100/900")
end

do
  local ledger = load()
  wow.sell[6303] = 750
  wow.noAuctionator()
  fish(ledger, 6303, "Raw Glossy Mightfish")
  check("without auctionator it falls back to the sell price", ledger.summary().gold, 750)
end

do
  local ledger = load()
  wow.prices[13756] = 5000
  Warcry.casting = false
  wow.now = wow.now + 600
  ledger.loot(wow.link(13756, "Raw Summer Bass"), 1)
  check("loot outside a cast is not fishing", ledger.summary().catches, 0)
end

do
  local ledger = load()
  check("an hourly rate needs a minute of fishing", ledger.perHour(3600, 30), 0)
  check("an hourly rate over one hour is the total", ledger.perHour(3600, 3600), 3600)
  check("an hourly rate over half an hour doubles", ledger.perHour(1800, 1800), 3600)
end

do
  local ledger = load()
  wow.prices[1] = 10
  wow.prices[2] = 900
  wow.prices[3] = 50
  fish(ledger, 1, "Sickly Fish")
  fish(ledger, 2, "Golden Fish")
  fish(ledger, 3, "Middling Fish")
  local top = ledger.summary().best
  check("the best fish is listed first", top[1].name, "Golden Fish")
  check("the worst fish is listed last", top[3].name, "Sickly Fish")
  check("every fish is listed", #top, 3)
end

do
  local ledger = load()
  wow.prices[13756] = 400
  wow.subzone = "Steamwheedle Port"
  fish(ledger, 13756, "Raw Summer Bass")
  fish(ledger, 13756, "Raw Summer Bass")
  local s = ledger.summary()
  check("the zone is recorded", s.zone, "Steamwheedle Port")
  check("a session is saved for the desktop", #WarcryLedgerDB.sessions > 0, true)
end

print(string.format("\n%d checks, %d failures", checks, failures))
os.exit(failures == 0 and 0 or 1)
