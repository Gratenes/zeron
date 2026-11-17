//! The composer footer's ring cluster. The plan-usage ring shows how much of
//! the active account's rate limit the session's harness has used, beside
//! the context ring. Clicking it opens the harness's accounts — each with its
//! usage meters — and clicking one switches to it, the same `ActivateAgentAccount` Settings →
//! Accounts runs. Both views share [`AccountsSnapshotCache`], so a switch in
//! either shows up in the other.
use std::time::{Duration, Instant};

use gpui::{
    Context, Entity, IntoElement, Render, SharedString, Subscription, Task, Window, div,
    prelude::*, px,
};
use zeron_proto::{AgentAccount, AgentAccountsSnapshot, AgentAuthKind, ChatAccountPin, HarnessId};
use zeron_rpc::methods;

use crate::popover;
use crate::settings::accounts::{
    self, AccountsSnapshotCache, UsageLevel, render_usage_meter, reports_usage, signs_in,
    usage_color, usage_level,
};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

/// Forced probes hit the provider; the engine throttles them too, but the
/// ring re-probes on every hover, so don't even ask more often than this.
const FORCE_MIN_INTERVAL: Duration = Duration::from_secs(30);
/// Background re-probe while a composer is alive: usage moves as turns run.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// The binding limit: the most-used window of the account. Pure.
pub fn used_fraction(account: &AgentAccount) -> Option<f32> {
    account
        .usage_windows
        .iter()
        .map(|window| window.used_fraction.clamp(0.0, 1.0))
        .reduce(f32::max)
}

/// Whether a Cursor model draws from Cursor's own pool (Composer and the Grok
/// models, the plan's `autoBucketModels`) rather than the third-party API pool
/// (Claude, GPT, Gemini, ...). Pure.
fn cursor_model_is_first_party(model: &str) -> bool {
    ["default", "auto", "composer", "vega", "grok", "cursor-grok"]
        .iter()
        .any(|prefix| {
            model
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '.']))
        })
}

/// What the footer ring reads: for Cursor, the window of the pool the selected
/// model draws from (first-party "Cursor" or third-party "Other"); otherwise,
/// or while that window is missing, the most-used window. Pure.
pub fn ring_fraction(
    account: &AgentAccount,
    harness: HarnessId,
    model: Option<&str>,
) -> Option<f32> {
    if harness == HarnessId::Cursor
        && let Some(model) = model
    {
        let label = if cursor_model_is_first_party(model) {
            zeron_proto::CURSOR_USAGE_CURSOR
        } else {
            zeron_proto::CURSOR_USAGE_OTHER
        };
        if let Some(window) = account.usage_windows.iter().find(|w| w.label == label) {
            return Some(window.used_fraction.clamp(0.0, 1.0));
        }
    }
    used_fraction(account)
}

/// The harness's live account, when it reports usage. Pure.
pub fn active_account(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
) -> Option<&AgentAccount> {
    snapshot
        .accounts
        .iter()
        .find(|account| account.harness == harness && account.active)
}

/// Agents whose chats can each run on a saved account of their own (the
/// engine's pins), so the picker chooses for the chat, not for the device.
pub fn chat_pinnable(harness: HarnessId) -> bool {
    matches!(harness, HarnessId::ClaudeCode | HarnessId::Muse)
}

/// One line of a chat's account choice: an account, or `None` for Automatic.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatChoice {
    pub account: Option<AgentAccount>,
    pub selected: bool,
}

/// What a chat can be given to run on: Automatic first, then the saved
/// accounts that can run a chat. A login that needs signing in again is
/// hidden (and so is an API key: spreading over it saves nothing) unless the
/// chat is already pinned to it, so it can still be seen and changed. Pure.
pub fn chat_choices(accounts: &[&AgentAccount], pin: &ChatAccountPin) -> Vec<ChatChoice> {
    let mut rows = vec![ChatChoice {
        account: None,
        selected: pin.account.is_none(),
    }];
    for account in accounts {
        let selected = pin.account.as_deref() == Some(account.id.as_str());
        let hidden = account.auth_kind == Some(AgentAuthKind::ApiKey)
            || (account.needs_signin && !selected);
        if !hidden {
            rows.push(ChatChoice {
                account: Some((*account).clone()),
                selected,
            });
        }
    }
    rows
}

/// "Automatic", or the account's email. Pure.
fn account_name(account: &AgentAccount) -> String {
    account
        .email
        .clone()
        .or_else(|| account.display_name.clone())
        .unwrap_or_else(|| "Unknown account".into())
}

/// The heading of a chat's account choice: "This chat: …". Pure.
pub fn chat_heading(accounts: &[&AgentAccount], pin: &ChatAccountPin) -> String {
    let name = pin
        .account
        .as_deref()
        .and_then(|id| accounts.iter().find(|a| a.id == id))
        .map(|a| account_name(a))
        .unwrap_or_else(|| "Automatic".into());
    format!("This chat: {name}")
}

/// Loads (and switches) the accounts of the composer's target device.
pub struct AccountUsage {
    state: Entity<AppState>,
    /// `None` = this device, as in [`AccountsSnapshotCache`].
    target: Option<String>,
    harness: Option<HarnessId>,
    /// The chat the composer is in, and the account it is pinned to (once the
    /// engine said): the picker chooses for this chat.
    chat: Option<String>,
    chat_pin: Option<(String, ChatAccountPin)>,
    /// The composer's selected model: Cursor's ring follows its pool.
    model: Option<String>,
    /// Once any list has been asked for the current target.
    loaded: bool,
    last_forced: Option<Instant>,
    error: Option<SharedString>,
    load_task: Option<Task<()>>,
    action_task: Option<Task<()>>,
    pin_task: Option<Task<()>>,
    popup: popover::Popup<FooterCard>,
    _poll: Task<()>,
    _cache: Subscription,
    _state: Subscription,
}

impl AccountUsage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                if this
                    .update(cx, |usage, cx| {
                        if usage.harness.is_some() {
                            usage.load(true, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            _state: cx.observe(&state, |_, _, cx| cx.notify()),
            state,
            target: None,
            harness: None,
            chat: None,
            chat_pin: None,
            model: None,
            loaded: false,
            last_forced: None,
            error: None,
            load_task: None,
            action_task: None,
            pin_task: None,
            popup: popover::Popup::default(),
            _poll: poll,
            // Settings → Accounts writes the same cache.
            _cache: cx.observe_global::<AccountsSnapshotCache>(|_, cx| cx.notify()),
        }
    }

    /// Point at the session's harness and device; loads on first sight of a
    /// device. Cheap enough to call every render.
    pub fn track(
        &mut self,
        harness: Option<HarnessId>,
        model: Option<String>,
        target: Option<String>,
        chat: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let harness = harness.filter(|h| signs_in(*h) && reports_usage(*h));
        let chat_changed = self.chat != chat || self.harness != harness;
        self.harness = harness;
        self.chat = chat;
        self.model = model;
        if self.target != target {
            self.target = target;
            self.loaded = false;
            self.last_forced = None;
            self.error = None;
        }
        if harness.is_some() && !self.loaded {
            self.loaded = true;
            self.load(true, cx);
        }
        if chat_changed || self.target_changed_for_pin() {
            self.load_chat_pin(cx);
        }
    }

    /// The pin read is for another chat than the one shown.
    fn target_changed_for_pin(&self) -> bool {
        self.chat_pin.as_ref().map(|(chat, _)| chat) != self.chat.as_ref() && self.chat.is_some()
    }

    /// Whether the picker chooses for the chat: its agent has pins and the
    /// engine answered for it.
    fn chat_mode(&self) -> Option<(&str, &ChatAccountPin)> {
        let harness = self.harness.filter(|h| chat_pinnable(*h))?;
        let _ = harness;
        let chat = self.chat.as_deref()?;
        self.chat_pin
            .as_ref()
            .filter(|(pinned_chat, _)| pinned_chat == chat)
            .map(|(chat, pin)| (chat.as_str(), pin))
    }

    /// Ask the engine which account the chat is pinned to. An engine that
    /// doesn't know the question (older) leaves the picker as it was.
    fn load_chat_pin(&mut self, cx: &mut Context<Self>) {
        let (Some(harness), Some(chat)) = (self.harness.filter(|h| chat_pinnable(*h)), self.chat.clone())
        else {
            self.chat_pin = None;
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = self.params(serde_json::json!({ "chatId": chat, "harness": harness }));
        self.pin_task = Some(cx.spawn(async move |this, cx| {
            let pin = engine
                .client()
                .call(methods::GET_CHAT_ACCOUNT_PIN, params)
                .await
                .ok()
                .and_then(|value| serde_json::from_value::<ChatAccountPin>(value).ok());
            this.update(cx, |usage, cx| {
                // Only for the chat still shown.
                if usage.chat.as_deref() == Some(chat.as_str()) {
                    usage.chat_pin = pin.map(|pin| (chat, pin));
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The person's choice for this chat: `Some(account)` pins it there,
    /// `None` is Automatic. The row flips at once and the engine's reply
    /// replaces it; a refusal restores it and says why.
    fn pin_chat(&mut self, choice: Option<&AgentAccount>, cx: &mut Context<Self>) {
        let (Some(harness), Some(chat)) = (self.harness.filter(|h| chat_pinnable(*h)), self.chat.clone())
        else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let previous = self.chat_pin.clone();
        self.chat_pin = Some((
            chat.clone(),
            ChatAccountPin {
                account: choice.map(|a| a.id.clone()),
                by_user: choice.is_some(),
            },
        ));
        self.error = None;
        let params = self.params(serde_json::json!({
            "chatId": chat,
            "harness": harness,
            "accountId": choice.map(|a| a.id.clone()),
        }));
        self.pin_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::PIN_CHAT_ACCOUNT, params)
                .await;
            this.update(cx, |usage, cx| {
                match result.map(serde_json::from_value::<ChatAccountPin>) {
                    Ok(Ok(pin)) => usage.chat_pin = Some((chat, pin)),
                    Ok(Err(_)) => usage.load_chat_pin(cx),
                    Err(err) => {
                        usage.chat_pin = previous;
                        usage.error = Some(err.to_string().into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn snapshot<'a>(&self, cx: &'a gpui::App) -> Option<&'a AgentAccountsSnapshot> {
        cx.try_global::<AccountsSnapshotCache>()?
            .0
            .get(&self.target)
    }

    fn params(&self, mut value: serde_json::Value) -> serde_json::Value {
        if let (Some(target), Some(object)) = (&self.target, value.as_object_mut()) {
            object.insert("targetDeviceId".into(), serde_json::json!(target));
        }
        value
    }

    /// Plain list first when nothing is cached (the engine's persisted usage
    /// paints at once), then the forced probe replaces it.
    fn load(&mut self, force_usage: bool, cx: &mut Context<Self>) {
        if force_usage {
            if self
                .last_forced
                .is_some_and(|at| at.elapsed() < FORCE_MIN_INTERVAL)
            {
                return;
            }
            self.last_forced = Some(Instant::now());
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let key = self.target.clone();
        let paint_first = force_usage && self.snapshot(cx).is_none();
        let plain = self.params(serde_json::json!({ "forceUsage": false }));
        let params = self.params(serde_json::json!({ "forceUsage": force_usage }));
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let fetch = |params: serde_json::Value| {
                let engine = engine.clone();
                async move {
                    engine
                        .client()
                        .call(methods::LIST_AGENT_ACCOUNTS, params)
                        .await
                        .ok()
                        .and_then(|value| {
                            serde_json::from_value::<AgentAccountsSnapshot>(value).ok()
                        })
                }
            };
            let store = |snapshot: AgentAccountsSnapshot, cx: &mut gpui::AsyncApp| {
                let key = key.clone();
                this.update(cx, |_, cx| {
                    cx.default_global::<AccountsSnapshotCache>()
                        .0
                        .insert(key, snapshot);
                })
                .ok();
            };
            if paint_first && let Some(snapshot) = fetch(plain).await {
                store(snapshot, cx);
            }
            if let Some(snapshot) = fetch(params).await {
                store(snapshot, cx);
            }
        }));
    }

    /// Switch optimistically, like Settings → Accounts: the rows flip at once
    /// and the engine's reply replaces them; a refusal restores the list.
    fn switch(&mut self, account: &AgentAccount, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let key = self.target.clone();
        let cache = &mut cx.default_global::<AccountsSnapshotCache>().0;
        let previous = cache.get(&key).cloned();
        if let Some(snapshot) = cache.get_mut(&key) {
            // Per-provider agents keep one live login per provider: only the
            // switched-to row's group changes.
            accounts::mark_switched(snapshot, account);
        }
        self.error = None;
        let params = self.params(serde_json::json!({
            "id": account.id,
            "accountId": account.id,
            "harness": account.harness,
        }));
        self.action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::ACTIVATE_AGENT_ACCOUNT, params)
                .await;
            this.update(cx, |usage, cx| {
                match result.map(serde_json::from_value::<AgentAccountsSnapshot>) {
                    Ok(Ok(snapshot)) => {
                        cx.default_global::<AccountsSnapshotCache>()
                            .0
                            .insert(key, snapshot);
                    }
                    Ok(Err(_)) => usage.load(false, cx),
                    Err(err) => {
                        if let Some(previous) = previous {
                            cx.default_global::<AccountsSnapshotCache>()
                                .0
                                .insert(key, previous);
                        }
                        usage.error = Some(err.to_string().into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The ring's reading: the live account's window for the selected model
    /// (the most-used one when the harness doesn't split by model), when the
    /// harness has a live account with usage to show.
    fn fraction(&self, cx: &gpui::App) -> Option<f32> {
        let harness = self.harness?;
        let snapshot = self.snapshot(cx)?;
        // A chat on an account of its own shows that account's window.
        let own = self
            .chat_mode()
            .and_then(|(_, pin)| pin.account.as_deref())
            .and_then(|id| snapshot.accounts.iter().find(|a| a.harness == harness && a.id == id));
        ring_fraction(
            own.or_else(|| active_account(snapshot, harness))?,
            harness,
            self.model.as_deref(),
        )
    }

    /// A trigger click: open this ring's popover, or close it when the
    /// press found it open (the card's mouse-down-out already began that
    /// close — see `Popup::note_trigger_press`).
    fn toggle(&mut self, card: FooterCard, cx: &mut Context<Self>) {
        if self.popup.take_press_was_open() || self.popup.as_open() == Some(&card) {
            self.dismiss(cx);
            return;
        }
        if card == FooterCard::Accounts {
            // Opening the card is the moment someone cares: re-probe.
            self.load(true, cx);
            self.load_chat_pin(cx);
        }
        self.popup.open(card);
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.popup.begin_close() {
            popover::reap_popup(cx, |usage: &mut Self| &mut usage.popup);
        }
        cx.notify();
    }

    /// A footer ring that opens `card` above itself, right-aligned like the
    /// model picker.
    fn trigger(
        &self,
        chip: gpui::Stateful<gpui::Div>,
        card: FooterCard,
        content: impl FnOnce(&Self, &mut Context<Self>) -> gpui::Div,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let chip = chip
            .relative()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |usage, _, _, _| {
                    usage
                        .popup
                        .note_trigger_press_matching(|open| *open == card);
                }),
            )
            .on_click(cx.listener(move |usage, _, _, cx| usage.toggle(card, cx)));
        if self.popup.get() != Some(&card) {
            return chip.into_any_element();
        }
        let content = content(self, cx)
            .on_mouse_down_out(cx.listener(|usage, _, _, cx| usage.dismiss(cx)))
            .into_any_element();
        chip.child(popover::anchored_menu_above_end(
            card.id(),
            content,
            self.popup.closing_since(),
        ))
        .into_any_element()
    }

    fn accounts_card(&self, cx: &mut Context<Self>) -> gpui::Div {
        let theme = &Theme::of(cx).for_popup();
        let harness = self.harness;
        let now = chrono::Utc::now();
        let rows: Vec<AgentAccount> = match (harness, self.snapshot(cx)) {
            (Some(harness), Some(snapshot)) => accounts::provider_accounts(snapshot, harness)
                .into_iter()
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        // A chat of an agent with pins chooses its own account here; the
        // device's login is switched in Settings → Accounts.
        if let Some((_, pin)) = self.chat_mode() {
            let refs: Vec<&AgentAccount> = rows.iter().collect();
            let heading = chat_heading(&refs, pin);
            let choices = chat_choices(&refs, pin);
            return popover::popover_card(theme)
                .w(px(400.0))
                .flex()
                .flex_col()
                .child(popover::menu_heading(theme, &heading))
                .children(choices.into_iter().enumerate().map(|(ix, choice)| {
                    let pick = choice.account.clone();
                    let row = popover::menu_row(
                        theme,
                        choice.selected,
                        format!("account-usage-choice-{ix}"),
                    )
                    .id(("account-usage-choice", ix))
                    .on_click(cx.listener(move |usage, _, _, cx| {
                        usage.pin_chat(pick.as_ref(), cx);
                    }));
                    match &choice.account {
                        None => row.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.5))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .child(SharedString::from("Automatic")),
                                )
                                .child(crate::settings::widgets::meta_line(
                                    theme,
                                    vec![
                                        div()
                                            .child(SharedString::from(
                                                "Spread over your accounts",
                                            ))
                                            .into_any_element(),
                                    ],
                                )),
                        ),
                        Some(account) => {
                            let mut meta: Vec<gpui::AnyElement> = account
                                .plan_label
                                .iter()
                                .map(|plan| {
                                    div().child(SharedString::from(plan.clone())).into_any_element()
                                })
                                .collect();
                            if choice.selected {
                                meta.push(
                                    div()
                                        .text_color(theme.accent)
                                        .child(SharedString::from("This chat"))
                                        .into_any_element(),
                                );
                            }
                            if account.needs_signin {
                                meta.push(
                                    div()
                                        .text_color(theme.danger)
                                        .child(SharedString::from("Signed out — sign in again"))
                                        .into_any_element(),
                                );
                            }
                            let (identity, meters) =
                                account_row_parts(theme, account, meta, ix, now);
                            row.child(identity).child(meters)
                        }
                    }
                }))
                .child(
                    div()
                        .px(px(8.0))
                        .py(px(4.0))
                        .text_size(px(11.5))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(
                            "The login in use on this device is switched in Settings → Accounts.",
                        )),
                )
                .children(self.error.clone().map(|error| {
                    div()
                        .px(px(8.0))
                        .py(px(4.0))
                        .text_size(px(12.0))
                        .text_color(theme.danger)
                        .child(error)
                }));
        }
        let title = format!(
            "{} accounts",
            harness.map_or("Agent", accounts::provider_name)
        );
        popover::popover_card(theme)
            .w(px(400.0))
            .flex()
            .flex_col()
            .child(popover::menu_heading(theme, &title))
            .children(rows.iter().enumerate().map(|(ix, account)| {
                let can_switch = !account.active && account.switchable;
                let mut meta: Vec<gpui::AnyElement> = account
                    .plan_label
                    .iter()
                    .map(|plan| {
                        div()
                            .child(SharedString::from(plan.clone()))
                            .into_any_element()
                    })
                    .collect();
                if account.active {
                    meta.push(
                        div()
                            .text_color(theme.accent)
                            .child(SharedString::from("In use"))
                            .into_any_element(),
                    );
                } else if let Some(reason) = account
                    .usage_error
                    .clone()
                    .filter(|_| account.usage_windows.is_empty())
                {
                    meta.push(div().child(SharedString::from(reason)).into_any_element());
                }
                let switch_to = account.clone();
                let (identity, meters) = account_row_parts(theme, account, meta, ix, now);
                popover::menu_row(theme, account.active, format!("account-usage-row-{ix}"))
                    .id(("account-usage-row", ix))
                    .when(!can_switch, |row| row.cursor_default())
                    .when(can_switch, |row| {
                        row.on_click(cx.listener(move |usage, _, _, cx| {
                            usage.switch(&switch_to, cx);
                        }))
                    })
                    .child(identity)
                    .child(meters)
            }))
            .children(self.error.clone().map(|error| {
                div()
                    .px(px(8.0))
                    .py(px(4.0))
                    .text_size(px(12.0))
                    .text_color(theme.danger)
                    .child(error)
            }))
    }
}

/// The two halves of an account's row: who it is (with `meta` beneath), and
/// its usage meters.
fn account_row_parts(
    theme: &Theme,
    account: &AgentAccount,
    mut meta: Vec<gpui::AnyElement>,
    ix: usize,
    now: chrono::DateTime<chrono::Utc>,
) -> (gpui::Div, gpui::Stateful<gpui::Div>) {
    let email: SharedString = account_name(account).into();
    if let Some(note) = crate::settings::accounts::stale_usage_note(account, now) {
        meta.push(div().child(SharedString::from(note)).into_any_element());
    }
    let identity = div()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .truncate()
                .text_size(px(12.5))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(email),
        )
        .when(!meta.is_empty(), |el| {
            el.child(crate::settings::widgets::meta_line(theme, meta))
        });
    let meters = div()
        .id(("account-usage-meters", ix))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .when(!account.usage_windows.is_empty(), |meters| {
            meters.tooltip(widgets::text_tooltip(accounts::usage_resets(
                &account.usage_windows,
                now,
            )))
        })
        .children(
            account
                .usage_windows
                .iter()
                .map(|window| render_usage_meter(window, theme)),
        );
    (identity, meters)
}

/// Which footer ring's popover is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FooterCard {
    Accounts,
    Context,
}

impl FooterCard {
    fn id(self) -> &'static str {
        match self {
            FooterCard::Accounts => "account-usage-menu",
            FooterCard::Context => "context-usage-menu",
        }
    }
}

/// The footer's ring cluster: account usage (accent arc, so the two read
/// apart), then context occupancy. Each opens its popover on click.
impl Render for AccountUsage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let context = self.state.read(cx).context_usage;
        let account = self.fraction(cx).map(|fraction| {
            let level = usage_level(fraction);
            let chip = crate::context_usage::ring_chip(
                "account-usage",
                fraction,
                usage_color(level, &theme),
                match level {
                    UsageLevel::Normal => theme.text_muted,
                    _ => usage_color(level, &theme),
                },
                format!("{}%", (fraction * 100.0).round() as u32),
                self.popup.get() == Some(&FooterCard::Accounts),
                &theme,
            );
            self.trigger(
                chip,
                FooterCard::Accounts,
                |usage, cx| usage.accounts_card(cx),
                cx,
            )
        });
        let context = crate::context_usage::has_window(context).then(|| {
            let chip = crate::context_usage::chip(
                context,
                self.popup.get() == Some(&FooterCard::Context),
                &theme,
            );
            self.trigger(
                chip,
                FooterCard::Context,
                move |_, cx| crate::context_usage::card(context, &Theme::of(cx).for_popup()),
                cx,
            )
        });
        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .children(account)
            .children(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::AgentUsageWindow;

    fn account(harness: HarnessId, active: bool, used: &[f32]) -> AgentAccount {
        serde_json::from_value(serde_json::json!({
            "id": format!("{harness:?}-{active}"),
            "harness": harness,
            "email": null,
            "planLabel": null,
            "active": active,
            "switchable": true,
            "usageWindows": used
                .iter()
                .map(|fraction| AgentUsageWindow {
                    label: "5h".into(),
                    used_fraction: *fraction,
                    resets_at: None,
                })
                .collect::<Vec<_>>(),
        }))
        .unwrap()
    }

    #[test]
    fn ring_shows_the_most_used_window() {
        assert_eq!(used_fraction(&account(HarnessId::Codex, true, &[])), None);
        assert_eq!(
            used_fraction(&account(HarnessId::Codex, true, &[0.12, 0.64])),
            Some(0.64)
        );
        assert_eq!(
            used_fraction(&account(HarnessId::Codex, true, &[1.4])),
            Some(1.0)
        );
    }

    #[test]
    fn cursor_ring_follows_the_selected_models_pool() {
        let mut cursor = account(HarnessId::Cursor, true, &[0.5, 0.01, 0.48]);
        for (window, label) in cursor.usage_windows.iter_mut().zip([
            zeron_proto::CURSOR_USAGE_CURSOR,
            zeron_proto::CURSOR_USAGE_GROK,
            zeron_proto::CURSOR_USAGE_OTHER,
        ]) {
            window.label = label.into();
        }
        let ring = |model| ring_fraction(&cursor, HarnessId::Cursor, model);
        assert_eq!(ring(Some("composer-2.5")), Some(0.5));
        assert_eq!(ring(Some("grok-4.7")), Some(0.5));
        assert_eq!(ring(Some("default")), Some(0.5));
        assert_eq!(ring(Some("claude-opus-5-5")), Some(0.48));
        assert_eq!(ring(Some("gpt-5.5")), Some(0.48));
        // "grokker" is not a Grok model; no model yet falls back to the most used.
        assert_eq!(ring(Some("grokker-1")), Some(0.48));
        assert_eq!(ring(None), Some(0.5));
        // Other harnesses ignore the model, and a missing pool falls back too.
        let codex = account(HarnessId::Codex, true, &[0.12, 0.64]);
        assert_eq!(
            ring_fraction(&codex, HarnessId::Codex, Some("gpt-5.5")),
            Some(0.64)
        );
        let lone = account(HarnessId::Cursor, true, &[0.3]);
        assert_eq!(
            ring_fraction(&lone, HarnessId::Cursor, Some("gpt-5.5")),
            Some(0.3)
        );
    }

    fn saved(id: &str, email: &str, needs_signin: bool, api_key: bool) -> AgentAccount {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "harness": "claude-code",
            "email": email,
            "planLabel": null,
            "active": false,
            "switchable": true,
            "needsSignin": needs_signin,
            "authKind": if api_key { "api-key" } else { "oauth" },
        }))
        .unwrap()
    }

    #[test]
    fn a_chat_is_offered_automatic_and_the_accounts_that_can_run_it() {
        let (a, dead, key) = (
            saved("a", "a@example.com", false, false),
            saved("dead", "dead@example.com", true, false),
            saved("key", "key@example.com", false, true),
        );
        let accounts = [&a, &dead, &key];
        let ids = |pin: &ChatAccountPin| -> Vec<(Option<String>, bool)> {
            chat_choices(&accounts, pin)
                .into_iter()
                .map(|c| (c.account.map(|a| a.id), c.selected))
                .collect()
        };
        // Not pinned: Automatic is the choice; a login that needs signing in
        // and an API key are not offered.
        assert_eq!(ids(&ChatAccountPin::default()), vec![(None, true), (Some("a".into()), false)]);
        // Pinned: that account is the choice, Automatic is not.
        let pinned = ChatAccountPin { account: Some("a".into()), by_user: true };
        assert_eq!(ids(&pinned), vec![(None, false), (Some("a".into()), true)]);
        // A chat already on a login that has since died still sees it, to change it.
        let stuck = ChatAccountPin { account: Some("dead".into()), by_user: true };
        assert_eq!(
            ids(&stuck),
            vec![(None, false), (Some("a".into()), false), (Some("dead".into()), true)]
        );
    }

    #[test]
    fn the_heading_says_which_account_this_chat_runs_on() {
        let a = saved("a", "a@example.com", false, false);
        let accounts = [&a];
        assert_eq!(chat_heading(&accounts, &ChatAccountPin::default()), "This chat: Automatic");
        let pinned = ChatAccountPin { account: Some("a".into()), by_user: true };
        assert_eq!(chat_heading(&accounts, &pinned), "This chat: a@example.com");
        // An account that is gone reads as Automatic, as the engine runs it.
        let gone = ChatAccountPin { account: Some("zz".into()), by_user: true };
        assert_eq!(chat_heading(&accounts, &gone), "This chat: Automatic");
    }

    #[test]
    fn only_agents_with_pins_choose_an_account_per_chat() {
        assert!(chat_pinnable(HarnessId::ClaudeCode));
        assert!(chat_pinnable(HarnessId::Muse));
        assert!(!chat_pinnable(HarnessId::Codex));
        assert!(!chat_pinnable(HarnessId::Cursor));
    }

    #[test]
    fn active_account_is_scoped_to_the_harness() {
        let snapshot = AgentAccountsSnapshot {
            accounts: vec![
                account(HarnessId::ClaudeCode, true, &[0.3]),
                account(HarnessId::Codex, false, &[0.1]),
                account(HarnessId::Codex, true, &[0.2]),
            ],
            warnings: Vec::new(),
        };
        assert_eq!(
            active_account(&snapshot, HarnessId::Codex).map(|a| a.id.as_str()),
            Some("Codex-true")
        );
        assert!(active_account(&snapshot, HarnessId::Cursor).is_none());
    }
}
