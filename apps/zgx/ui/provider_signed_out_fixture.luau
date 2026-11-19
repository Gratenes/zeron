-- Settings → Providers with a login that is dead (its refresh was refused:
-- the engine flags it `needs_signin`), mounted on its own with agents made
-- up here (no engine in a test host): what tests/provider_signed_out.test.luau
-- drives. Claude Code has the login in use and a saved one that is dead;
-- Pi (a login per model provider) has a live Claude one and a dead one. The
-- dead rows keep their last meters, taken `as_of` three hours ago. The
-- host's calls are answered as it would: what was asked, in order, is
-- `signed_out_calls`. Here, beside app.luau, as a mounted script loads
-- modules from its own directory down.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
-- Addresses plain to start: the tests read them.
ds.PrivateText.hidden:set(false)
-- Settings' host calls (the page's settings dialog, its toasts).
require("./settings_stub")
local agents = require("@account/agents")
local registry = require("@account/registry") :: any
registry.load = function() end
local providers = require("@app/screens/settings/providers")

local THREE_HOURS = 3 * 3600

local function account(id: string, email: string, plan: string, provider: string?, active: boolean, dead: boolean): any
	return {
		id = id,
		email = email,
		plan = plan,
		session = 0.04,
		week = 0.28,
		active = active,
		switchable = true,
		provider = provider,
		needs_signin = if dead then true else nil,
		as_of = if dead then os.time() - THREE_HOURS else nil,
	}
end

local function stale_account(id: string, email: string, plan: string): any
	local a = account(id, email, plan, nil, false, false)
	a.as_of = os.time() - 2 * 3600
	return a
end

local function agent(id: string, name: string, mark: string, accounts: { any }): any
	return {
		id = id,
		name = name,
		mark = mark,
		installed = true,
		can_install = true,
		enabled = true,
		version = "1.0.0",
		installing = false,
		login = "idle",
		accounts = accounts,
	}
end

local model: { any } = {
	agent("claude-code", "Claude Code", "claude", {
		account("cl1", "live@example.com", "Max", nil, true, false),
		account("cl2", "dead@example.com", "Max", nil, false, true),
		-- Rate-limited, not dead: its meters are the last reading, two hours old.
		stale_account("cl3", "limited@example.com", "Max"),
	}),
	agent("pi", "Pi", "pi", {
		account("p1", "pi-live@example.com", "Claude Max", "anthropic", true, false),
		account("p2", "pi-dead@example.com", "Claude Max", "anthropic", false, true),
	}),
}

local function publish()
	local list = {}
	for _, a in model do
		local item = table.clone(a)
		local accounts = {}
		for _, acc in a.accounts do
			table.insert(accounts, table.clone(acc))
		end
		item.accounts = accounts
		if a.id == "pi" then
			item.groups = { { id = "anthropic", name = "Anthropic", accounts = accounts, login = "Claude", key = "Anthropic" } }
		end
		table.insert(list, item)
	end
	agents.now:set({ state = "ready", list = list } :: any)
end

local said: { string } = {}
local calls = ui.state("signed_out_calls", "")
local function note(what: string)
	table.insert(said, what)
	calls:set(table.concat(said, ","))
end

local stub = agents :: any
stub.load = function() end
stub.target = function() end
stub.sign_in = function(id: string)
	note(`sign_in:{id}`)
end
stub.sign_in_to = function(id: string, provider: string)
	note(`sign_in_to:{id}:{provider}`)
end
stub.cancel = function() end
stub.dismiss = function() end
stub.set_enabled = function() end
stub.switch = function(id: string, account_id: string): any
	note(`switch:{id}:{account_id}`)
	return { after = function(_: any, done: (any) -> ()) done(nil) end }
end
stub.remove = function(id: string, account_id: string)
	note(`remove:{id}:{account_id}`)
end

publish()

return ui.column {
	w_full = true,
	h_full = true,
	ds.toast.Host {},
	ui.scroll {
		id = "page",
		w_full = true,
		h_full = true,
		bg = "$zeron.window",
		ui.column { w_full = true, px = "$space.6", py = "$space.6", providers.view() },
	},
}
