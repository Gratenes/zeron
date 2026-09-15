use std::{rc::Rc, time::Duration};

use gpui::{AnyWindowHandle, App, AppContext as _, Entity};
use serde::Deserialize;
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, Request, RequestInit, Response};
use zeron_ui::{
    shell,
    state::{AppState, EngineHandle},
};

use crate::rpc::connection::{ConnectionEpochs, connect_client};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserAuth {
    token: String,
    device_id: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EngineDevice {
    device_id: String,
    #[serde(default)]
    owner: bool,
    #[serde(default)]
    revoked_at: Option<String>,
}

pub struct BrowserSession {
    state: Entity<AppState>,
    _window: AnyWindowHandle,
}

impl BrowserSession {
    pub fn start(state: Entity<AppState>, window: AnyWindowHandle, cx: &mut App) -> Rc<Self> {
        let session = Rc::new(Self {
            state,
            _window: window,
        });
        let runner = session.clone();
        cx.spawn(async move |cx| runner.run(cx).await).detach();
        session
    }

    pub fn handle(&self, _action: shell::ExternalLifecycleAction) {
        if let Some(window) = web_sys::window() {
            let _ = window.location().reload();
        }
    }

    async fn run(self: Rc<Self>, cx: &mut gpui::AsyncApp) {
        let mut epochs = ConnectionEpochs::default();
        loop {
            let Some(auth) = browser_auth() else {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                continue;
            };
            let epoch = epochs.begin_auth();
            match connect_engine(epoch, &auth).await {
                Ok(handle) if epochs.is_current(epoch) => {
                    let mut closed = handle.client().watch_closed();
                    self.state.clone().update(cx, |state, cx| {
                        state.attach_engine(handle, cx);
                        state.apply_auth(zeron_proto::AuthState::SignedIn {
                            user: zeron_proto::UserProfile {
                                id: auth.device_id.clone(),
                                email: "paired-browser@local".into(),
                                name: Some("Paired browser".into()),
                            },
                            org_id: None,
                        });
                    });
                    let _ = closed.changed().await;
                    self.state.clone().update(cx, |state, cx| {
                        let _ = state.detach_engine(cx);
                    });
                }
                Ok(handle) => handle.shutdown().await,
                Err(error) => web_sys::console::error_1(&JsValue::from_str(&error)),
            }
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
        }
    }
}

fn browser_auth() -> Option<BrowserAuth> {
    let window = web_sys::window()?;
    let value = js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("__KRATOS_AUTH")).ok()?;
    if value.is_null() || value.is_undefined() {
        return None;
    }
    serde_wasm_bindgen::from_value(value).ok()
}

async fn connect_engine(
    epoch: crate::rpc::connection::ConnectionEpoch,
    auth: &BrowserAuth,
) -> Result<EngineHandle, String> {
    let devices: Vec<EngineDevice> = get_json("/pair/engines", &auth.token).await?;
    let engine = devices
        .into_iter()
        .find(|device| device.owner && device.revoked_at.is_none())
        .ok_or_else(|| "No active owner engine device is available".to_string())?;
    let connected = connect_client(epoch, &engine.device_id, &auth.token).await?;
    let url = connected.url().to_string();
    EngineHandle::from_connected_client(connected.into_client(), url)
        .await
        .map_err(|error| error.to_string())
}

async fn get_json<T: for<'de> Deserialize<'de>>(path: &str, token: &str) -> Result<T, String> {
    let headers = Headers::new().map_err(|_| "Cannot create request headers")?;
    headers
        .set("authorization", &format!("Bearer {token}"))
        .map_err(|_| "Cannot set bearer header")?;
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_headers(&headers);
    let request = Request::new_with_str_and_init(path, &init)
        .map_err(|_| format!("Cannot create request for {path}"))?;
    let window = web_sys::window().ok_or("Browser window unavailable")?;
    let response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|_| format!("{path} request failed"))?
        .dyn_into::<Response>()
        .map_err(|_| format!("{path} returned an invalid response"))?;
    if !response.ok() {
        if response.status() == 401 {
            reject_browser_auth();
        }
        return Err(format!("{path} returned HTTP {}", response.status()));
    }
    let value = JsFuture::from(
        response
            .json()
            .map_err(|_| format!("{path} returned invalid JSON"))?,
    )
    .await
    .map_err(|_| format!("{path} returned invalid JSON"))?;
    serde_wasm_bindgen::from_value(value).map_err(|error| error.to_string())
}

fn reject_browser_auth() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(callback) = js_sys::Reflect::get(
        window.as_ref(),
        &JsValue::from_str("__KRATOS_AUTH_REJECTED"),
    ) else {
        return;
    };
    if let Some(callback) = callback.dyn_ref::<js_sys::Function>() {
        let _ = callback.call0(window.as_ref());
    }
}
