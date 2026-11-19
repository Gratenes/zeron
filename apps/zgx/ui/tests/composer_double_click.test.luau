-- A double click in the composer's draft selects the word, a triple the
-- line: the pill's own press handler (the hairline's) once sent the caret to
-- the draft's end on every press that bubbled up from the draft, undoing the
-- draft's own selection.

local BOX = 'role:textbox "Message"'

local function composer(go: string, text: string): any
	local app = mount("chat_fixture.luau")
	app:resize(1000, 800)
	app:command(go)
	app:frames(20)
	app:click(BOX)
	app:fill(BOX, text)
	app:frames(5)
	return app
end

local WORD = string.rep("w", 50)

for _, go in { "go_s2", "go_home" } do
	test(`{go}: a double click on a word in the draft selects it`, function()
		local app = composer(go, WORD)
		app:double_click(BOX)
		app:frames(5)
		local from, to = app:selection(BOX)
		expect(`{from}..{to}`).to_be(`0..{#WORD}`)
	end)
end

test("a click in the draft puts the caret there, not at its end", function()
	local app = composer("go_s2", WORD)
	app:click(BOX)
	app:frames(5)
	local from, to = app:selection(BOX)
	expect(from == to and from > 0 and from < #WORD).to_be(true)
end)
