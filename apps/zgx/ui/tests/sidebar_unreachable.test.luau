-- A chat whose device is away (the engine publishes the device `online =
-- false`: its presence heartbeat went quiet) wears a wifi-off glyph in its
-- sidebar row, saying which device and what it means; back online, it's
-- gone. View options → Show → Unreachable chats (on by default) turns it
-- off. On the shell in ui/shell_fixture.luau (devbox hosts Borealis's chats).

local SIDE = "#shell-sidebar"
local SHOW = "screens/shell/sidebar.luau::shell_sidebar_show_unreachable"
local SAYS = "devbox is offline — this chat can't run until it reconnects"

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	return app
end

local function glyphs(app: any): number
	local n = 0
	for line in app:tree(SIDE):gmatch("[^\n]+") do
		if line:find(SAYS, 1, true) then
			n += 1
		end
	end
	return n
end

test("it's on by default, and a device that's online marks nothing", function()
	local app = open()
	expect(app:state(SHOW)).to_be(true)
	expect(glyphs(app)).to_be(0)
end)

test("every chat of an offline device says so; the others' don't; back online, none", function()
	local app = open()
	app:command("fixture.devbox_offline")
	app:frames(40)
	-- (Borealis's two chats, on devbox.)
	expect(glyphs(app)).to_be(2)
	app:command("fixture.devbox_online")
	app:frames(40)
	expect(glyphs(app)).to_be(0)
end)

test("Show → Unreachable chats off hides it; on, it's back", function()
	local app = open()
	app:command("fixture.devbox_offline")
	app:frames(40)
	app:click(`{SIDE} >> role:button "Sidebar view options"`)
	app:frames(20)
	expect(app:has('role:checkbox "Unreachable chats"')).to_be(true)
	app:click('role:checkbox "Unreachable chats"')
	app:frames(30)
	expect(app:state(SHOW)).to_be(false)
	app:key("Escape")
	app:frames(30)
	expect(glyphs(app)).to_be(0)
	app:set_state(SHOW, true)
	app:frames(30)
	expect(glyphs(app)).to_be(2)
end)

-- Nested side chats (Show → Nest side chats): Fix the build is on this
-- computer, its one side chat on devbox.
local NEST = "screens/shell/sidebar.luau::shell_sidebar_side_chats"
-- (The tree clips a long tooltip: its start is enough.)
local SIDE_SAYS = "A side chat is on an offline device"
local GROUP_SAYS = "A chat in here is on an offline"

local function count(app: any, words: string): number
	local n = 0
	for line in app:tree(SIDE):gmatch("[^\n]+") do
		if line:find(words, 1, true) then
			n += 1
		end
	end
	return n
end

local function nested(): any
	local app = open()
	app:command("side_chat_on_devbox")
	app:set_state(NEST, true)
	app:command("fixture.devbox_offline")
	app:frames(40)
	return app
end

test("a nested side chat on an offline device wears the glyph; its folded parent shows it for them", function()
	local app = nested()
	-- Folded (as they start): the parent says a side chat is away; Borealis's
	-- two own chats say their device.
	expect(count(app, SIDE_SAYS)).to_be(1)
	expect(glyphs(app)).to_be(2)
	app:click(`{SIDE} >> role:button "Show side chats"`)
	app:frames(40)
	-- Opened: the side chat's own row has it, the parent's is gone.
	expect(count(app, SIDE_SAYS)).to_be(0)
	expect(glyphs(app)).to_be(3)
	app:command("fixture.devbox_online")
	app:frames(40)
	expect(glyphs(app)).to_be(0)
end)

test("a folded group's tile says a chat in it is unreachable; unfolded, its chats do", function()
	local app = nested()
	-- (Ship the release, asking for you, is archived: a state's dot would
	-- take the tile's badge, and stays on a folded group.)
	app:right_click(`{SIDE} >> role:button "Ship the release"`)
	app:frames(20)
	app:click(`role:menuitem "Archive"`)
	app:frames(30)
	expect(count(app, GROUP_SAYS)).to_be(0)
	app:click(`{SIDE} >> role:button "Borealis"`)
	app:frames(40)
	expect(count(app, GROUP_SAYS)).to_be(1)
	expect(glyphs(app)).to_be(0)
	app:click(`{SIDE} >> role:button "Borealis"`)
	app:frames(40)
	expect(count(app, GROUP_SAYS)).to_be(0)
	expect(glyphs(app)).to_be(1)
end)

-- Where a row's glyph and its chevron stand: the unreachable glyph is left
-- of the side chats' chevron.
local function x_of(app: any, row: string, words: string): number
	for line in app:tree(`{SIDE} >> role:button "{row}"`):gmatch("[^\n]+") do
		if line:find(words, 1, true) then
			return tonumber(line:match(" (%-?%d+),%-?%d+ %d+x%d+")) :: number
		end
	end
	error(`no "{words}" on "{row}"`)
end

test("a row's unreachable glyph is left of its side chats' chevron", function()
	local app = nested()
	-- (Folded: the glyph says its side chat is away.)
	expect(x_of(app, "Fix the build", SIDE_SAYS) < x_of(app, "Fix the build", "Show side chats")).to_be(true)
end)
