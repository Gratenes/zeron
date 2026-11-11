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

local sessions = require("@account/sessions")

local M = {}

local WAITING = { input = true, done = true, failed = true }

function M.count(chats: { any }): number
	local n = 0
	for _, c in chats do
		-- (`parent`: not a main chat, whatever the host sent it as.)
		if c.parent == nil and (WAITING[c.status] or c.woke ~= nil) then
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
		return M.count(now.chats)
	end)
	ui.effect(function()
		local n = count:get()
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
