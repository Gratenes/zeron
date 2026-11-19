-- The composer's Account list dates the figures a failed re-check left: an
-- account whose usage check was rate-limited says why and how old its last
-- reading is ("… · Usage as of 2 hours ago"); a current one carries no date.

local function window(label: string, used: number): any
	return { label = label, used = used }
end

local function with_accounts(list: { any }): any
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	app:frames(30)
	app:set_state("private_text_hidden", false)
	app:set_state("agents.on_device", { [""] = { ["claude-code"] = list } })
	app:frames(10)
	return app
end

local function open_list(app: any)
	app:click('role:button "Model"')
	app:frames(30)
	app:click("#model-picker-account")
	app:frames(30)
end

test("an account whose usage check was rate-limited says how old its figures are", function()
	local app = with_accounts({
		{ id = "a1", email = "wing@example.test", plan = "Max", active = true, can_switch = true, windows = { window("5h", 0.34) } },
		{
			id = "a2", email = "work@example.test", plan = "Pro", active = false, can_switch = true,
			windows = { window("5h", 0.05) },
			error = "Usage check rate-limited by Anthropic — retrying in 30m",
			as_of = os.time() - 2 * 3600,
		},
	})
	open_list(app)
	expect(app:has('~"Pro · Usage check rate-limited by Anthropic — retrying in 30m · Usage as of 2 hours ago"')).to_be(true)
end)

test("the one in use, whose last re-check failed, dates its figures after them", function()
	local app = with_accounts({
		{
			id = "a1", email = "wing@example.test", plan = "Max", active = true, can_switch = true,
			windows = { window("5h", 0.34), window("Week", 0.12) },
			error = "Usage check rate-limited by Anthropic",
			as_of = os.time() - 3 * 3600,
		},
		{ id = "a2", email = "work@example.test", plan = "Pro", active = false, can_switch = true, windows = { window("5h", 0.05) } },
	})
	open_list(app)
	expect(app:has('~"Max · 5h 34% · Week 12% · Usage as of 3 hours ago"')).to_be(true)
	-- (A current account's line has no date.)
	expect(app:has('~"Pro · 5h 5%"')).to_be(true)
end)
