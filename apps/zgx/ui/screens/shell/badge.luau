-- The app icon's badge: how many of the person's chats wait for them. A red
-- count on the taskbar button (Windows), on the Dock tile (macOS), and where a
-- Linux dock shows one; the system's own (`ui.badge`, the app's and not a
-- plugin's), taken off at none.
--
--   badge.count(chats)  how many of `chats` (`sessions.now`'s `chats`) wait
--   badge.install()     in the shell: keeps the icon's badge at that count
--
-- A chat waits when it asks something of the person (status "input": a
-- question, an approval, a review), has finished since they last looked
-- ("done"), has failed since ("failed"), or woke from its sleep and hasn't
-- been looked at since (`woke`). Working chats, and chats waiting on work
-- that will wake their agent ("waiting"), don't: nothing is the person's
-- yet. Each chat counts once, whatever else is true of it, and archived
-- chats (not in `chats`) and a chat's side chats and subagents (`children`)
-- not at all. Reading a chat marks it seen on every device
-- (`sessions.seen`), and that takes it off the count.
--
-- Settings → Notifications can turn it off (`prefs.badge`: off takes the
-- badge down at once, nothing to restart) or count only the chats that ask
-- (`prefs.badge_count` "needs_me": status "input"; the default, "all", is
-- everything above).

local prefs = require("@app/screens/settings/prefs")
local sessions = require("@account/sessions")

local M = {}

local WAITING = { input = true, done = true, failed = true }
local ASKING = { input = true }

-- `mode`: "all" (the default) or "needs_me" (only the chats that ask).
function M.count(chats: { any }, mode: string?): number
	local n = 0
	for _, c in chats do
		-- (`parent`: not a main chat, whatever the host sent it as.)
		if c.parent ~= nil then
			continue
		end
		if mode == "needs_me" then
			if ASKING[c.status] then
				n += 1
			end
		elseif WAITING[c.status] or c.woke ~= nil then
			n += 1
		end
	end
	return n
end

function M.install()
	-- The count once the engine has said, else nothing: loading or offline,
	-- the chats are a guess, and the badge stays as it was.
	local count = ui.computed(function(): number?
		local now = sessions.now:get()
		if now.state ~= "ready" then
			return nil
		end
		return M.count(now.chats, prefs.badge_count:get())
	end)
	ui.effect(function()
		-- (Off, none: whatever the engine is saying.)
		local on = prefs.badge:get()
		local n = if on then count:get() else 0
		-- (The window's activity read too: told again as it comes forward,
		-- as a taskbar button made after the first telling, or one made
		-- again with Explorer, hasn't the badge. It's the same again,
		-- otherwise.)
		local _ = ui.window.active:get()
		if n == nil then
			return
		end
		-- (Refused, by a guard on the app's own VM or by the system: the
		-- badge isn't worth an error.)
		pcall(ui.badge, n)
	end)
end

return M
