-- The composer's account row, for an agent whose chats each run on an account
-- of their own (Claude Code, Muse): "This chat", Automatic and the accounts
-- that can run a chat, the choice pinning THIS chat (`pin_chat`) and never
-- switching the device's login; a login that needs signing in again shown and
-- not offered; and, where the engine hasn't said the chat's pin (an older
-- one, or Cloud's vault), the row as it always was.

local function window(label: string, used: number): any
	return { label = label, used = used }
end

local function accounts(): { any }
	return {
		{ id = "a1", email = "wing@example.test", plan = "Max", active = true, can_switch = true, windows = { window("5h", 0.34), window("Week", 0.12) } },
		{ id = "a2", email = "work@example.test", plan = "Pro", active = false, can_switch = true, windows = { window("5h", 0.05), window("Week", 0.91) } },
	}
end

-- What the engine said of the open chat's pin, on this computer (the chat the
-- fixture opens first is s1).
local function pinned(account: string?, by_user: boolean?): { [string]: any }
	return { [""] = { ["s1:claude-code"] = { account = account, by_user = by_user == true } } }
end

local function mounted(list: { any }, pins: { [string]: any }?): any
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	app:frames(30)
	app:set_state("private_text_hidden", false)
	app:set_state("agents.on_device", { [""] = { ["claude-code"] = list } })
	if pins then
		app:set_state("agents.chat_pins", pins)
	end
	app:frames(10)
	return app
end

local function open_row(app: any)
	app:click('role:button "Model"')
	app:frames(30)
	app:click("#model-picker-account")
	app:frames(30)
end

test("a chat's pin is asked after, as the chat is first seen", function()
	local app = mounted(accounts())
	expect(app:state("calls")).to_contain("watch_chat::s1:claude-code")
end)

test("a chat on automatic reads 'This chat: Automatic' and lists it first", function()
	local app = mounted(accounts(), pinned(nil))
	app:click('role:button "Model"')
	app:frames(30)
	local row = app:tree("#model-picker-account")
	expect(row).to_contain("This chat")
	expect(row).to_contain("Automatic")
	app:click("#model-picker-account")
	app:frames(30)
	expect(app:has('~"Automatic"')).to_be(true)
	expect(app:has('~"wing@example.test"')).to_be(true)
	expect(app:has('~"work@example.test"')).to_be(true)
end)

test("a chat on automatic still shows the usage of the account in use: the chip's ring and the row's figure", function()
	local app = mounted(accounts(), pinned(nil))
	expect(app:has("#model-picker-usage")).to_be(true)
	app:click('role:button "Model"')
	app:frames(30)
	local row = app:tree("#model-picker-account")
	expect(row).to_contain("Automatic")
	-- (The fuller of the account in use's two windows: wing's 34%.)
	expect(row).to_contain("34%")
	-- (The account in use isn't named as this chat's.)
	expect(string.find(row, "wing@example.test", 1, true)).to_be(nil)
end)

test("a chat pinned to an account says which, with its usage", function()
	local app = mounted(accounts(), pinned("a2", true))
	-- (Its ring is on the chip still.)
	expect(app:has("#model-picker-usage")).to_be(true)
	app:click('role:button "Model"')
	app:frames(30)
	local row = app:tree("#model-picker-account")
	expect(row).to_contain("This chat")
	expect(row).to_contain("work@example.test")
	-- (The fuller of its two windows, not the account in use's.)
	expect(row).to_contain("91%")
	expect(string.find(row, "wing@example.test", 1, true)).to_be(nil)
end)

test("choosing an account pins this chat and never switches the device's login", function()
	local app = mounted(accounts(), pinned(nil))
	open_row(app)
	app:click('~"work@example.test"')
	app:frames(30)
	local calls = app:state("calls")
	expect(calls).to_contain("pin_chat::s1:claude-code:a2")
	expect(string.find(calls, "switch_on", 1, true)).to_be(nil)
end)

test("choosing Automatic takes the pin back", function()
	local app = mounted(accounts(), pinned("a2", true))
	open_row(app)
	app:click('~"Automatic"')
	app:frames(30)
	expect(app:state("calls")).to_contain("pin_chat::s1:claude-code:")
	expect(string.find(app:state("calls"), "pin_chat::s1:claude-code:a", 1, true)).to_be(nil)
end)

test("a login that needs signing in again is shown signed out and can't be chosen", function()
	local list = accounts()
	list[2].needs_signin = true
	list[2].detail = nil
	local app = mounted(list, pinned(nil))
	open_row(app)
	expect(app:has('~"Pro · Signed out — sign in again"')).to_be(true)
	app:click('~"work@example.test"')
	app:frames(30)
	local calls = app:state("calls")
	expect(string.find(calls, "pin_chat", 1, true)).to_be(nil)
	expect(string.find(calls, "switch_on", 1, true)).to_be(nil)
	-- It can be signed in again, where Settings → Providers has it.
	expect(app:has('~"Sign in again…"')).to_be(true)
	app:click('~"Sign in again…"')
	app:frames(30)
	expect(app:state("calls")).to_contain("target:")
end)

test("with no signed-out account there is nothing to sign in again", function()
	local app = mounted(accounts(), pinned(nil))
	open_row(app)
	expect(app:has('~"Sign in again…"')).to_be(false)
end)

test("opening the panel asks after the chat's pin again, with the usage", function()
	local app = mounted(accounts(), pinned(nil))
	app:click('role:button "Model"')
	app:frames(30)
	expect(app:state("calls")).to_contain("refresh_chat::s1:claude-code")
	expect(app:state("calls")).to_contain("refresh_device:")
end)

test("no pin said for the chat (an older engine): the row chooses the device's login, as it did", function()
	local app = mounted(accounts())
	app:click('role:button "Model"')
	app:frames(30)
	local row = app:tree("#model-picker-account")
	expect(string.find(row, "This chat", 1, true)).to_be(nil)
	expect(row).to_contain("wing@example.test")
	app:click("#model-picker-account")
	app:frames(30)
	expect(app:has('~"Automatic"')).to_be(false)
	app:click('~"work@example.test"')
	app:frames(30)
	expect(app:state("calls")).to_contain("switch_on::claude-code:a2")
end)

test("a chat on a Cloud machine chooses among the vault's accounts for the device, never a pin", function()
	local app = mounted(accounts(), pinned(nil))
	app:set_state("agents.on_device", {
		[""] = { ["claude-code"] = accounts() },
		cloud = { ["claude-code"] = { { id = "v1", email = "team@example.test", plan = "Max", active = true, can_switch = true, windows = {} } } },
	})
	app:set_state("chat_device", "cloud-f60cde46")
	app:frames(10)
	app:click('role:button "Model"')
	app:frames(30)
	local row = app:tree("#model-picker-account")
	expect(string.find(row, "This chat", 1, true)).to_be(nil)
	expect(row).to_contain("team@example.test")
	expect(string.find(app:state("calls"), "watch_chat:cloud", 1, true)).to_be(nil)
end)
