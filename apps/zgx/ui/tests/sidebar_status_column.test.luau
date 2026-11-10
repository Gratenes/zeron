-- A row's status items (the subagents pill, the unreachable glyph) end at
-- one column, the fold chevron's left edge, on every row: with a chevron or
-- without (its slot stays, empty), pill or glyph, and the pill's right edge
-- is the glyph's. A row with both has them side by side, the glyph last. The
-- gaps between the trailing items are 4 points, and the row's inset at its
-- end 4. On the shell in ui/shell_fixture.luau (`status_column`, with devbox
-- away).

local SIDE = "#shell-sidebar"
local NEST = "screens/shell/sidebar.luau::shell_sidebar_side_chats"
local WAY = "devbox is offline"

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("status_column")
	app:set_state(NEST, true)
	app:command("fixture.devbox_offline")
	app:frames(40)
	return app
end

-- A row's box, by its node's words ("subagent" for the pill, the
-- tooltip's start for the glyph, `Show side chats` for the chevron):
-- left, right.
local function edges(app: any, title: string, words: string): (number, number)
	for line in app:tree(`{SIDE} >> role:button "{title}"`):gmatch("[^\n]+") do
		if line:find(words, 1, true) and not line:find("text ", 1, true) and not line:find(" img ", 1, true) then
			local x, w = line:match(" (%-?%d+),%-?%d+ (%d+)x%d+")
			return tonumber(x) :: number, tonumber(x) + tonumber(w)
		end
	end
	error(`no "{words}" on "{title}"`)
end

test("a pill ends at the same column on a row with a chevron and one without", function()
	local app = open()
	local _, with = edges(app, "Fix the build", "subagent")
	local _, without = edges(app, "Write the docs", "subagent")
	expect(without).to_be(with)
	-- (And ends 4 before the chevron's slot: the chevron's left edge.)
	local chevron = edges(app, "Fix the build", "Show side chats")
	expect(chevron - with).to_be(4)
end)

test("the glyph ends at the same column on a row with a chevron and one without", function()
	local app = open()
	local _, with = edges(app, "Tidy the tests", WAY)
	local _, without = edges(app, "Answer the review", WAY)
	expect(without).to_be(with)
end)

test("the pill's right edge is the glyph's", function()
	local app = open()
	local _, pill = edges(app, "Fix the build", "subagent")
	local _, glyph = edges(app, "Tidy the tests", WAY)
	expect(pill).to_be(glyph)
end)

test("a row with both has the pill, then the glyph, ending at that column", function()
	local app = open()
	local _, pill = edges(app, "Ship the release", "subagent")
	local glyph_left, glyph = edges(app, "Ship the release", WAY)
	local _, alone = edges(app, "Tidy the tests", WAY)
	expect(glyph).to_be(alone)
	expect(glyph_left - pill).to_be(4)
end)

test("the corner sits 4 from the chevron's slot and the row's end", function()
	local app = open()
	local _, chevron_end = edges(app, "Tidy the tests", "Show side chats")
	for line in app:tree(`{SIDE} >> role:button "Tidy the tests"`):gmatch("[^\n]+") do
		if line:find(' text "2h"', 1, true) then
			local x, w = line:match(" (%-?%d+),%-?%d+ (%d+)x%d+")
			-- (A 2h time, right-aligned in the 26-wide corner: its left a little in.)
			expect(tonumber(x) - chevron_end >= 4).to_be(true)
			local _, row_end = edges(app, "Tidy the tests", "Tidy the tests")
			expect(row_end - (tonumber(x) + tonumber(w))).to_be(4)
			return
		end
	end
	error("no time on Tidy the tests")
end)

-- The mixed cases: a side chat can run on another device than its parent.
local function mixed(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("mixed_status")
	app:set_state(NEST, true)
	app:command("fixture.devbox_offline")
	app:frames(40)
	return app
end

local SIDE_WAY = "A side chat is on an offline"

test("a parent online with a pill and a folded side chat on an offline device: the pill, then the side glyph, at the column", function()
	local app = mixed()
	local _, pill = edges(app, "Fix the build", "subagent")
	local glyph_left, glyph = edges(app, "Fix the build", SIDE_WAY)
	local chevron = edges(app, "Fix the build", "Show side chats")
	expect(glyph + 4).to_be(chevron)
	expect(glyph_left - pill).to_be(4)
	-- (The column the other rows end at.)
	local _, alone = edges(app, "Tidy the tests", WAY)
	expect(glyph).to_be(alone)
end)

test("a parent offline with a working side chat on an online device: the pill, then the glyph, in the same order", function()
	local app = mixed()
	local _, pill = edges(app, "Tidy the tests", "subagent")
	local glyph_left, glyph = edges(app, "Tidy the tests", WAY)
	local chevron = edges(app, "Tidy the tests", "Show side chats")
	expect(glyph + 4).to_be(chevron)
	expect(glyph_left - pill).to_be(4)
end)

test("unfolded, the offline side chat's own row has the glyph at the column, and the parent's side glyph is gone", function()
	local app = mixed()
	local chevron = edges(app, "Fix the build", "Show side chats")
	app:click(`{SIDE} >> role:button "Fix the build" >> role:button "Show side chats"`)
	app:frames(40)
	local _, glyph = edges(app, "Side on devbox", WAY)
	expect(glyph + 4).to_be(chevron)
	local has_side = false
	for line in app:tree(`{SIDE} >> role:button "Fix the build"`):gmatch("[^\n]+") do
		if line:find(SIDE_WAY, 1, true) then
			has_side = true
		end
	end
	expect(has_side).to_be(false)
end)
