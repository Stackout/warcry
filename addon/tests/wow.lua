local wow = {}

wow.now = 1000
wow.subzone = "Thousand Needles"
wow.zone = "Tanaris"
wow.prices = {}
wow.sell = {}
wow.auctionator = true

function GetTime() return wow.now end
function time() return 1700000000 end
function GetSubZoneText() return wow.subzone end
function GetZoneText() return wow.zone end
function GetCoinTextureString(copper) return tostring(copper) .. "c" end
unpack = table.unpack
string.trim = function(s) return (string.gsub(s, "^%s*(.-)%s*$", "%1")) end
STANDARD_TEXT_FONT = "font"
UIParent = {}

C_Timer = { After = function(_, fn) fn() end }

local function stubFrame()
  local f = {}
  setmetatable(f, { __index = function() return function() return f end end })
  f.IsShown = function() return false end
  return f
end

function CreateFrame() return stubFrame() end

local function linkFor(id, name)
  return "|cffffffff|Hitem:" .. id .. "::::::::1:::::::|h[" .. name .. "]|h|r"
end

function C_Item_GetItemInfo(link)
  local id = tonumber(string.match(link, "item:(%d+)"))
  local name = string.match(link, "%[(.-)%]")
  return name, link, 1, 1, 1, "", "", 1, "", "", wow.sell[id] or 0
end

C_Item = { GetItemInfo = C_Item_GetItemInfo }
GetItemInfo = C_Item_GetItemInfo

local function makeAuctionator()
  return {
  API = {
    v1 = {
      GetAuctionPriceByItemLink = function(caller, link)
        assert(type(caller) == "string" and caller ~= "", "callerID must be a non-empty string")
        local id = tonumber(string.match(link, "item:(%d+)"))
        return wow.prices[id]
      end,
      GetVendorPriceByItemLink = function(caller, link)
        assert(type(caller) == "string" and caller ~= "", "callerID must be a non-empty string")
        local id = tonumber(string.match(link, "item:(%d+)"))
        return wow.sell[id]
      end,
    },
  },
  }
end

wow.link = linkFor

function wow.noAuctionator()
  Auctionator = nil
end

function wow.reset()
  wow.now = 1000
  wow.prices = {}
  wow.sell = {}
  wow.subzone = "Thousand Needles"
  Auctionator = makeAuctionator()
  WarcryLedgerDB = nil
  Warcry = { casting = false, castEndedAt = 0, on = function() end, emit = function() end }
end

Auctionator = makeAuctionator()

return wow
