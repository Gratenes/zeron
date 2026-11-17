-- Claude rotation's window: tells its service whether the setting is on, and
-- says what the service did, in a toast. The work is the service's
-- (service.luau), which runs with no window open; this only tells you while
-- Zeron is open.

local zeron = require("./zeron")("claude-rotation")

local service = zeron.service()
local enabled = zeron.setting("enabled")

-- The service keeps what it was last told, so it follows the setting with
-- no window open too.
local function tell()
	service:call("set_enabled", { enabled = enabled:peek() ~= false }):after(function() end, function(err)
		zeron.set_status("enabled", { badge = "Not running", tone = "warning", text = `Zeron couldn't reach this plugin's service here ({zeron.reason(err)}).` })
	end)
end
tell()
zeron.on_setting(function(id: string)
	if id == "enabled" then
		zeron.set_status("enabled", nil)
		tell()
	end
end)

service:on("pinned", function(e)
	zeron.toast(`A new Claude Code chat is running on {e.to}.`)
end)

service:on("moved", function(e)
	zeron.toast(`A Claude Code chat moved to {e.to} for its next turn.`)
end)
