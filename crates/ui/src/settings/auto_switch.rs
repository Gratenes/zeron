//! Settings → Providers: the device's automatic account switching — a switch,
//! and the usage threshold it switches at (engine `agent_accounts::auto_switch`).
//! Device-addressed like the rest of the page; Cloud's account is the
//! vault's, so a Cloud target shows nothing.

use chrono::{DateTime, Local};
use gpui::{AnyElement, Context, Entity, Render, Task, Window, div, prelude::*, px};
use zeron_proto::{AccountAutoSwitch, AccountAutoSwitchStatus, AccountSwitchEvent};
use zeron_rpc::methods;

use super::widgets;
use crate::popover::Loadable;
use crate::state::AppState;
use crate::theme::Theme;

/// The thresholds offered, in percent.
const THRESHOLDS: [u8; 7] = [70, 75, 80, 85, 90, 95, 98];

/// The thresholds the menu lists: the offered ones, plus a saved one that
/// isn't among them (set by another client), so it stays shown. Pure.
fn threshold_options(current: u8) -> Vec<u8> {
    let mut options = THRESHOLDS.to_vec();
    if !options.contains(&current) {
        options.push(current);
        options.sort_unstable();
    }
    options
}

/// What the switch row says under its title. Pure.
fn describe(status: &AccountAutoSwitchStatus) -> String {
    match &status.last_switch {
        Some(event) if status.settings.enabled => last_switch_copy(event),
        _ => format!(
            "At {}% of a usage limit, switch to the saved account with the most left",
            status.settings.threshold_percent
        ),
    }
}

fn last_switch_copy(event: &AccountSwitchEvent) -> String {
    let name = zeron_proto::providers::provider_name(event.harness);
    let to = event.to_email.as_deref().unwrap_or("another account");
    let at = DateTime::from_timestamp_millis(event.at)
        .map(|at| at.with_timezone(&Local).format("%-I:%M %p").to_string())
        .unwrap_or_default();
    format!("Switched {name} to {to} at {at}")
}

pub(super) struct AutoSwitchCard {
    state: Entity<AppState>,
    /// `None` = this device.
    target: Option<String>,
    status: Loadable<AccountAutoSwitchStatus>,
    saving: bool,
    task: Option<Task<()>>,
    threshold_select: widgets::SelectState,
}

impl AutoSwitchCard {
    pub fn new(state: Entity<AppState>, target: Option<String>, cx: &mut Context<Self>) -> Self {
        let mut card = Self {
            state,
            target,
            status: Loadable::Idle,
            saving: false,
            task: None,
            threshold_select: widgets::SelectState::default(),
        };
        card.request(None, cx);
        card
    }

    fn targets_cloud(&self) -> bool {
        self.target.as_deref() == Some(zeron_proto::CLOUD_ACCOUNTS_DEVICE)
    }

    pub fn set_target(&mut self, target: Option<String>, cx: &mut Context<Self>) {
        widgets::close_select(self, |card: &mut Self| &mut card.threshold_select, cx);
        if self.target == target {
            return;
        }
        self.target = target;
        self.status = Loadable::Idle;
        self.task = None;
        self.saving = false;
        self.request(None, cx);
    }

    /// Read (or, with `save`, set) the device's setting. A save carries the
    /// status it replaced, restored if the device refuses it.
    fn request(
        &mut self,
        save: Option<(AccountAutoSwitch, AccountAutoSwitchStatus)>,
        cx: &mut Context<Self>,
    ) {
        if self.targets_cloud() {
            cx.notify();
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.status = Loadable::Error("Device is not connected".into());
            cx.notify();
            return;
        };
        self.saving = save.is_some();
        if !self.saving {
            self.status = Loadable::Loading;
        }
        let (method, mut params) = match &save {
            Some((settings, _)) => (
                methods::SET_ACCOUNT_AUTO_SWITCH,
                serde_json::to_value(settings).unwrap_or_default(),
            ),
            None => (methods::GET_ACCOUNT_AUTO_SWITCH, serde_json::json!({})),
        };
        params["targetDeviceId"] = serde_json::json!(self.target);
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(method, params)
                .await
                .map_err(|error| match error {
                    zeron_rpc::RpcError::UnknownMethod(_) => {
                        "Update Zeron on the selected device to switch accounts automatically."
                            .to_string()
                    }
                    _ => error.to_string(),
                })
                .and_then(|value| {
                    serde_json::from_value::<AccountAutoSwitchStatus>(value)
                        .map_err(|error| error.to_string())
                });
            this.update(cx, |card, cx| {
                card.saving = false;
                match result {
                    Ok(status) => card.status = Loadable::Ready(status),
                    // A refused save puts the switch back where it was.
                    Err(error) if save.is_some() => {
                        if let Some((_, previous)) = save {
                            card.status = Loadable::Ready(previous);
                        }
                        crate::toast::error(cx, error);
                    }
                    Err(error) => card.status = Loadable::Error(error),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn save(&mut self, settings: AccountAutoSwitch, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        // Optimistic: the reply replaces it.
        let Loadable::Ready(status) = &mut self.status else {
            return;
        };
        let previous = status.clone();
        status.settings = settings;
        self.request(Some((settings, previous)), cx);
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        let Some(status) = self.status.ready() else {
            return;
        };
        let settings = AccountAutoSwitch {
            enabled: !status.settings.enabled,
            ..status.settings
        };
        self.save(settings, cx);
    }

    fn set_threshold(&mut self, threshold_percent: u8, cx: &mut Context<Self>) {
        let Some(status) = self.status.ready() else {
            return;
        };
        let settings = AccountAutoSwitch {
            threshold_percent,
            ..status.settings
        };
        self.save(settings, cx);
    }

    fn render_threshold_row(
        &self,
        theme: &Theme,
        current: u8,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let options = threshold_options(current);
        let selected = options.iter().position(|t| *t == current).unwrap_or(0);
        let select = widgets::select(
            "account-auto-switch-threshold",
            "Switch at",
            theme,
            |card: &mut Self| &mut card.threshold_select,
        )
        .options(
            options
                .iter()
                .map(|t| widgets::SelectOption::new(format!("{t}%"))),
            selected,
        )
        .width(88.0)
        .on_select(move |card, ix, _, cx| {
            if let Some(threshold) = options.get(ix) {
                card.set_threshold(*threshold, cx);
            }
        })
        .render(&self.threshold_select, cx);
        widgets::card_row(theme, false)
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .child(widgets::row_title(theme, "Switch at"))
                    .child(widgets::meta_line(
                        theme,
                        vec![
                            div()
                                .child("The most-used window counts. Chats pick up the new account on their next turn.")
                                .into_any_element(),
                        ],
                    )),
            )
            .child(select)
            .into_any_element()
    }
}

impl Render for AutoSwitchCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.targets_cloud() {
            return div().into_any_element();
        }
        let theme = Theme::of(cx).for_settings_surface();
        let status = self.status.ready().cloned();
        let enabled = status.as_ref().is_some_and(|s| s.settings.enabled);
        let interactive = status.is_some() && !self.saving;
        let meta = match (&self.status, &status) {
            (_, Some(status)) => describe(status),
            (Loadable::Error(error), _) => error.clone(),
            _ => "Loading…".to_string(),
        };
        let switch_row = widgets::card_row(&theme, true)
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .child(widgets::row_title(&theme, "Switch accounts automatically"))
                    .child(widgets::meta_line(
                        &theme,
                        vec![div().truncate().child(meta).into_any_element()],
                    )),
            )
            .child(
                widgets::toggle_switch(&theme, enabled, "account-auto-switch")
                    .id("account-auto-switch-toggle")
                    .flex_none()
                    .role(gpui::Role::Switch)
                    .aria_label("Switch accounts automatically")
                    .aria_toggled(if enabled {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .when(interactive, |el| {
                        el.cursor_pointer()
                            .tab_index(0)
                            .focus_visible(|s| s.border_2().border_color(theme.accent))
                            .on_click(cx.listener(|card, _, _, cx| card.toggle(cx)))
                    })
                    .when(!interactive, |el| el.opacity(0.5)),
            );
        let threshold_row = status
            .filter(|s| s.settings.enabled)
            .map(|s| self.render_threshold_row(&theme, s.settings.threshold_percent, cx));
        widgets::section(
            &theme,
            "Accounts",
            widgets::section_card(&theme)
                .mt_0()
                .child(switch_row)
                .children(threshold_row),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_threshold_set_elsewhere_stays_listed() {
        assert_eq!(threshold_options(90), THRESHOLDS.to_vec());
        assert_eq!(threshold_options(93), vec![70, 75, 80, 85, 90, 93, 95, 98]);
    }

    #[test]
    fn the_row_names_the_threshold_until_a_switch_happens() {
        let mut status = AccountAutoSwitchStatus {
            settings: AccountAutoSwitch {
                enabled: true,
                threshold_percent: 85,
            },
            last_switch: None,
        };
        assert_eq!(
            describe(&status),
            "At 85% of a usage limit, switch to the saved account with the most left"
        );
        status.last_switch = Some(AccountSwitchEvent {
            harness: zeron_proto::HarnessId::Codex,
            provider: None,
            from_id: "a".into(),
            from_email: Some("a@example.com".into()),
            to_id: "b".into(),
            to_email: Some("b@example.com".into()),
            used_fraction: 0.9,
            at: 0,
        });
        assert!(
            describe(&status).starts_with("Switched Codex to b@example.com at "),
            "{}",
            describe(&status)
        );
    }
}
