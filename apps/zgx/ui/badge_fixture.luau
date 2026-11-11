-- The app icon's badge (screens/shell/badge): the shell (ui/shell_fixture.luau,
-- as it is) with the engine's chats made up here, one command a state of them.
-- What src/taskbar_badge_tests.rs drives; it reads the icon's badge from the
-- test desktop.

local node = require("./shell_fixture")
local prefs = require("@app/screens/settings/prefs")
local sessions = require("@account/sessions")

local NOW = os.time()

-- A main chat, as the host sends one.
local function chat(id: string, status: string?, woke: number?): any
	return {
		id = id,
		title = `Chat {id}`,
		project = "atlas",
		device = "mac",
		status = status,
		woke = woke,
		updated = NOW - 60,
		created = NOW - 3600,
		agent = "claude",
	}
end

local function say(state: string, chats: { any }, archived: { any }?, children: any?)
	local now = table.clone(sessions.now:peek()) :: any
	now.state = state
	now.chats = chats
	now.archived = archived or {}
	now.children = children or {}
	sessions.now:set(now)
end

-- Four wait (asks, finished, failed, woke); working, waiting on work, and
-- idle don't; nor an archived one that finished, nor a side chat and a
-- subagent asking (they are `children`, not `chats`).
local function mixed_chats(): { any }
	return {
		chat("asks", "input"),
		chat("finished", "done"),
		chat("failed", "failed"),
		chat("woke", nil, NOW - 30),
		chat("busy", "working"),
		chat("later", "waiting"),
		chat("quiet", nil),
	}
end
local function mixed_children(): any
	return {
		asks = {
			{ id = "side", parent = "asks", title = "A side chat", device = "mac", status = "input", updated = NOW, created = NOW },
			{ id = "sub", parent = "asks", title = "A subagent", device = "mac", status = "done", updated = NOW, created = NOW, child_kind = "subagent" },
		},
	}
end

ui.command {
	id = "badge.mixed",
	title = "Badge: a mix of chats",
	run = function()
		say("ready", mixed_chats(), { chat("shelved", "done") }, mixed_children())
	end,
}
-- The finished one looked at: seen, so its "done" goes (`sessions.seen`).
ui.command {
	id = "badge.finished_seen",
	title = "Badge: the finished one seen",
	run = function()
		local chats = mixed_chats()
		chats[2] = chat("finished", nil)
		say("ready", chats, { chat("shelved", "done") }, mixed_children())
	end,
}
-- Another asks.
ui.command {
	id = "badge.another_asks",
	title = "Badge: another chat asks",
	run = function()
		local chats = mixed_chats()
		chats[2] = chat("finished", nil)
		table.insert(chats, chat("more", "input"))
		say("ready", chats, nil, mixed_children())
	end,
}
-- All at work or quiet: none wait.
ui.command {
	id = "badge.none",
	title = "Badge: nothing waits",
	run = function()
		say("ready", { chat("busy", "working"), chat("later", "waiting"), chat("quiet", nil) }, nil, nil)
	end,
}
-- The engine not there (yet): the chats it has are a guess.
ui.command {
	id = "badge.offline",
	title = "Badge: the engine offline",
	run = function()
		say("offline", {}, nil, nil)
	end,
}

-- Settings → Notifications' two choices.
ui.command { id = "badge.pref_off", title = "Badge: setting off", run = function() prefs.badge:set(false) end }
ui.command { id = "badge.pref_on", title = "Badge: setting on", run = function() prefs.badge:set(true) end }
ui.command { id = "badge.pref_needs_me", title = "Badge: count only what asks", run = function() prefs.badge_count:set("needs_me") end }
ui.command { id = "badge.pref_all", title = "Badge: count asks and finished", run = function() prefs.badge_count:set("all") end }

return node
