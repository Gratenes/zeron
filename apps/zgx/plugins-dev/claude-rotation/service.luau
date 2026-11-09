-- Claude rotation's service: run by Zeron's engine on each device that
-- allowed it, with no window open. It decides which saved Claude Code account
-- a chat runs on, and never switches the active one: other chats and your own
-- terminals keep running on it.
--
--  * Right before a chat's first run Zeron asks (`run.preparing`); this
--    answers with the saved account that has the most left.
--  * A chat whose turn a limit stopped (`limit.hit`), or whose account signed
--    out, is pinned to another account, so its next turn moves.
--
-- An account is skipped while it is signed out or its limit hasn't reset
-- (`limited_until`). Among the rest the one with the lowest usage (its
-- fullest window) wins; accounts within 5 points of each other, or whose usage
-- Zeron hasn't read yet, are told apart by how many chats are pinned to them,
-- so a burst of new chats still takes turns. `zeron.pins()` is read each
-- time, not remembered, so the service can stop between chats.
--
-- The window half (init.luau) tells this whether the setting is on; with no
-- window ever having said, it is on.

local zeron = require("./zeron")("claude-rotation")

local PROVIDER = "claude-code"
-- Usage within this many points counts as level.
local LEVEL = 5

local enabled = zeron.store("enabled", true)

zeron.serve("set_enabled", function(args)
	enabled:set(args.enabled ~= false)
	return { enabled = enabled:get() }
end, { safe = true })

-- The fullest window's percent used (nil: no reading yet).
local function used(row: any): number?
	if row == nil or row.windows == nil then
		return nil
	end
	local top: number? = nil
	for _, w in row.windows do
		if top == nil or w.used > top then
			top = w.used
		end
	end
	return top
end

-- The saved accounts a chat can run on now, with what decides between them.
-- `away` leaves one out (the account a chat is moving off).
local function candidates(away: string?): { any }
	local accounts = zeron.accounts({ provider = PROVIDER }):await()
	local usage: { [string]: any } = {}
	for _, row in zeron.usage({ provider = PROVIDER }):await() do
		usage[row.account] = row
	end
	local chats: { [string]: number } = {}
	for _, pin in zeron.pins({ provider = PROVIDER }):await() do
		chats[pin.account] = (chats[pin.account] or 0) + 1
	end
	local out = {}
	for _, a in accounts do
		local row = usage[a.account]
		if a.pinnable and not a.signed_out and a.account ~= away and not (row ~= nil and row.limited_until ~= nil) then
			table.insert(out, { account = a.account, email = a.email, used = used(row), chats = chats[a.account] or 0 })
		end
	end
	return out
end

local function better(a: any, b: any): boolean
	if a.used ~= nil and b.used ~= nil and math.abs(a.used - b.used) >= LEVEL then
		return a.used < b.used
	end
	if a.chats ~= b.chats then
		return a.chats < b.chats
	end
	if a.used ~= nil and b.used ~= nil then
		return a.used < b.used
	end
	return false
end

-- The best account to run a chat on (nil: none to choose, so it stays where
-- it is).
local function choose(away: string?): any
	local best: any = nil
	for _, c in candidates(away) do
		if best == nil or better(c, best) then
			best = c
		end
	end
	return best
end

-- What a `run.preparing` listener returns is its answer: `{ account }`.
zeron.on("run.preparing", function(e)
	if not enabled:get() then
		return nil
	end
	local best = choose(nil)
	if best == nil then
		return nil
	end
	zeron.emit("pinned", { chat = e.chat, to = best.email or best.account })
	return { account = best.account }
end, { provider = PROVIDER })

-- Move one chat off `away` for its next turn.
local function move(chat: string, away: string?)
	local best = choose(away)
	if best == nil then
		return
	end
	zeron.pin_account({ provider = PROVIDER, chat = chat, account = best.account }):await()
	zeron.emit("moved", { chat = chat, to = best.email or best.account })
end

-- Where a chat runs now: the account it is pinned to (nil: the active one).
local function current(chat: string): string?
	for _, pin in zeron.pins({ provider = PROVIDER }):await() do
		if pin.chat == chat then
			return pin.account
		end
	end
	for _, a in zeron.accounts({ provider = PROVIDER }):await() do
		if a.active then
			return a.account
		end
	end
	return nil
end

-- Every chat pinned to `account` moves off it.
local function empty(account: string)
	for _, pin in zeron.pins({ provider = PROVIDER }):await() do
		if pin.account == account and pin.chat ~= nil then
			move(pin.chat, account)
		end
	end
end

-- A turn a limit stopped: that chat moves. An account that hit its limit:
-- the chats pinned to it move.
zeron.on("limit.hit", function(e)
	if not enabled:get() then
		return
	end
	if e.blocked_turn and e.chat ~= nil then
		move(e.chat, current(e.chat))
	elseif e.account ~= nil then
		empty(e.account)
	end
end, { provider = PROVIDER })

-- A login that needs signing in again: its chats move, or the one it stopped.
zeron.on("account.signed_out", function(e)
	if enabled:get() and e.account ~= nil then
		empty(e.account)
	end
end, { provider = PROVIDER })

zeron.on("login.expired", function(e)
	if enabled:get() and e.chat ~= nil then
		move(e.chat, current(e.chat))
	end
end, { provider = PROVIDER })
