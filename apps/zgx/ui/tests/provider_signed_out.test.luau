-- Settings → Providers for a saved login that is dead (the engine flags it
-- `needs_signin`: its refresh was refused): its row says "Signed out", dates
-- its last meters ("Usage as of 3 hours ago"), carries a "Sign in again"
-- button and menu entry that start its agent's sign-in (for its own model
-- provider where the agent keeps a login each), and is no switch target. A
-- healthy row has none of it. On ui/provider_signed_out_fixture.luau.

local PAGE = "#page"

local function open(): any
	local app = mount("provider_signed_out_fixture.luau")
	app:resize(900, 1600)
	app:frames(30)
	return app
end

local function calls(app: any): string
	return app:state("signed_out_calls") :: any
end

local function shows(app: any, scope: string, text: string): boolean
	return app:has(scope) and string.find(app:tree(scope), text, 1, true) ~= nil
end

-- The page's tree between two of its texts (Claude Code's card has no row
-- ids: its accounts are told apart by their address).
local function between(app: any, from: string, to: string): string
	local tree = app:tree(PAGE)
	local start = string.find(tree, from, 1, true) :: number
	local stop = string.find(tree, to, start, true) :: number
	return string.sub(tree, start, stop - 1)
end

test("a dead login is marked signed out, its meters dated, a healthy one is not", function()
	local app = open()
	local live = between(app, "live@example.com", "dead@example.com")
	local dead = between(app, "dead@example.com", "limited@example.com")
	expect(string.find(live, "In use", 1, true) ~= nil).to_be(true)
	expect(string.find(live, "Signed out", 1, true) == nil).to_be(true)
	expect(string.find(live, "Sign in again", 1, true) == nil).to_be(true)
	expect(string.find(dead, "Signed out", 1, true) ~= nil).to_be(true)
	expect(string.find(dead, "Usage as of 3 hours ago", 1, true) ~= nil).to_be(true)
	expect(string.find(dead, "Sign in again", 1, true) ~= nil).to_be(true)
	-- (Its last meters stay on the row: dimmed, not dropped.)
	expect(string.find(dead, "Week usage, 28%", 1, true) ~= nil).to_be(true)
end)

test("meters a failed re-check left are dated, on a row that is not signed out", function()
	local app = open()
	local limited = between(app, "limited@example.com", "pi-live@example.com")
	expect(string.find(limited, "Usage as of 2 hours ago", 1, true) ~= nil).to_be(true)
	expect(string.find(limited, "Signed out", 1, true) == nil).to_be(true)
	expect(string.find(limited, "Sign in again", 1, true) == nil).to_be(true)
	-- (Current meters carry no date.)
	local live = between(app, "live@example.com", "dead@example.com")
	expect(string.find(live, "Usage as of", 1, true) == nil).to_be(true)
end)

test("its Sign in again starts the agent's sign-in", function()
	local app = open()
	-- (Claude Code's is the first on the page.)
	app:click('role:button "Sign in again" [1]')
	app:frames(5)
	expect(calls(app)).to_be("sign_in:claude-code")
end)

test("its menu signs in again and offers no switch", function()
	local app = open()
	app:click('#account-row-p2 >> role:button "Account actions"')
	app:frames(5)
	local menu = app:tree("role:menu")
	expect(string.find(menu, 'button "Sign in again"', 1, true) ~= nil).to_be(true)
	expect(string.find(menu, 'button "Remove account"', 1, true) ~= nil).to_be(true)
	expect(string.find(menu, "Switch to this account", 1, true) == nil).to_be(true)
	app:click('role:menu >> role:button "Sign in again"')
	app:frames(5)
	expect(calls(app)).to_be("sign_in_to:pi:anthropic")
end)

test("a healthy saved login still offers its switch", function()
	local app = open()
	-- (The fixture's healthy rows are both in use: make one saved.)
	app:click('#account-row-p1 >> role:button "Account actions"')
	app:frames(5)
	local menu = app:tree("role:menu")
	expect(string.find(menu, "Sign in again", 1, true) == nil).to_be(true)
end)

test("for an agent with a login per provider it signs in that provider", function()
	local app = open()
	expect(shows(app, "#account-row-p2", "Signed out")).to_be(true)
	expect(shows(app, "#account-row-p1", "Signed out")).to_be(false)
	app:click('#account-row-p2 >> role:button "Sign in again"')
	app:frames(5)
	expect(calls(app)).to_be("sign_in_to:pi:anthropic")
end)
