-- A row's fold chevron (a click target) keeps one column: on rows with and
-- without the subagents pill, as the pill comes and goes, with a time of
-- "5m", "12h" or "104w" in the corner, with the pointer on the row and off
-- it, and with ⌘ held (its jump keys in the corner). What comes and goes
-- sits left of the chevron, and the corner has one fixed width. On the shell
-- in ui/shell_fixture.luau (`row_columns`).

local SIDE = "#shell-sidebar"
local NEST = "screens/shell/sidebar.luau::shell_sidebar_side_chats"

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("row_columns")
	app:set_state(NEST, true)
	app:frames(40)
	return app
end

-- How far in a row's chevron stands (its x), by the row's title.
local function chevron(app: any, title: string): number
	for line in app:tree(`{SIDE} >> role:button "{title}"`):gmatch("[^\n]+") do
		if line:find('button "Show side chats"', 1, true) then
			return tonumber(line:match(" (%-?%d+),%-?%d+ %d+x%d+")) :: number
		end
	end
	error(`no chevron on "{title}"`)
end

test("the chevron keeps its column on a row with the subagents pill and one without", function()
	local app = open()
	expect(chevron(app, "Fix the build")).to_be(chevron(app, "Write the docs"))
end)

test("the chevron stays put as the subagents pill goes", function()
	local app = open()
	local with = chevron(app, "Fix the build")
	app:command("fixture.subagents_done")
	app:frames(40)
	expect(chevron(app, "Fix the build")).to_be(with)
end)

test("the chevron keeps its column whatever the time in the corner: 5m, 12h, 104w", function()
	local app = open()
	local x = chevron(app, "Write the docs")
	expect(chevron(app, "Fix the build")).to_be(x)
	expect(chevron(app, "Tidy the tests")).to_be(x)
end)

test("the chevron stays put as the pointer arrives, and as ⌘ is held", function()
	local app = open()
	local x = chevron(app, "Write the docs")
	app:hover(`{SIDE} >> role:button "Write the docs"`)
	app:frames(20)
	expect(chevron(app, "Write the docs")).to_be(x)
	app:set_state("shell_jump_hints", true)
	app:frames(4)
	expect(chevron(app, "Write the docs")).to_be(x)
	expect(chevron(app, "Tidy the tests")).to_be(x)
end)

-- Its time's left edge past the chevron's right edge, on a row, in points:
-- the line's own gap and no more, at the widest resting time ("104w"); the
-- jump keys (wider) are drawn over the row and add none.
local function gap_to_time(app: any, title: string, time: string): number
	local right = 0
	local left = 0
	for line in app:tree(`{SIDE} >> role:button "{title}"`):gmatch("[^\n]+") do
		local x, w = line:match(" (%-?%d+),%-?%d+ (%d+)x%d+")
		if line:find('button "Show side chats"', 1, true) then
			right = tonumber(x) + tonumber(w)
		elseif line:find(` text "{time}"`, 1, true) then
			left = tonumber(x) :: number
		end
	end
	return left - right
end

test("the corner takes the room of its widest resting time, no more: the gap past the chevron is the line's own", function()
	local app = open()
	expect(gap_to_time(app, "Tidy the tests", "104w") <= 8).to_be(true)
	app:set_state("shell_jump_hints", true)
	app:frames(4)
	expect(gap_to_time(app, "Tidy the tests", "104w") <= 8).to_be(true)
end)
