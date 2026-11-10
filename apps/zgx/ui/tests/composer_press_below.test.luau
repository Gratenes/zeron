-- A press on the pill's hairline border, the last point under the draft's
-- line (ds/composer), gives the draft the keys, as one on its padding does:
-- the press layer is inside the border, so the pill itself takes the press.

local BOX = 'role:textbox "Message"'

local function has_keys(app: any): boolean
	for line in app:tree():gmatch("[^\n]+") do
		if line:find(" focused", 1, true) then
			return line:find('textbox "Message"', 1, true) ~= nil
		end
	end
	return false
end

local function bounds(app: any, target: string): (number, number, number, number)
	local line = string.match(app:tree(target), "[^\n]+") or ""
	local x, y, w, h = line:match(" (%-?%d+),(%-?%d+) (%d+)x(%d+)")
	return tonumber(x) or 0, tonumber(y) or 0, tonumber(w) or 0, tonumber(h) or 0
end

for _, go in { "go_s2", "go_home" } do
	test(`{go}: a press anywhere under the draft's line, to the pill's foot, gives it the keys`, function()
		local app = mount("chat_fixture.luau")
		app:resize(740, 400)
		app:command(go)
		app:frames(20)
		local _, cy, w = bounds(app, "#chat-composer")
		local _, by, _, dh = bounds(app, BOX)
		-- The pill's foot: its toolbar's top less the gap, on a session; a
		-- new chat's is its last row.
		for _, below in { 2, 6, 10, 11, 11.5, 12 } do
			app:click(BOX)
			app:key("Escape")
			app:frames(20)
			expect(has_keys(app)).to_be(false)
			app:click("#chat-composer", { x = w // 2, y = by - cy + dh + below })
			app:frames(20)
			expect(`{below} below the draft: {has_keys(app)}`).to_be(`{below} below the draft: true`)
		end
	end)
end
