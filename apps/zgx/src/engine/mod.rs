//! The Zeron engine this app works through: its account (src/auth.rs), the
//! agents on this Mac (src/agents.rs) and GitHub for Cloud (src/github.rs)
//! are all the engine's, reached over its local IPC ([`rpc`]). One
//! connection for every window, kept open: dialled again when it drops,
//! the engine started when nothing answers ([`manager`], which also says
//! which engine and refuses production's).
//!
//! Callers `call` a method (waiting a while for a connection first) or ask
//! to hear each new connection (`on_connect`) to start their watches on it.

pub mod manager;
pub mod owner;
pub mod rpc;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use serde_json::{Value as Json, json};
use zgx_desktop::{TaskSpawner, WindowContext};

use manager::{Config, Spawner};
use rpc::{DialError, Rpc};

/// How long a call waits for a connection before it says `offline`.
const CONNECT_WAIT: Duration = Duration::from_secs(20);
/// How long the port may refuse this app's key before it says so: a new
/// engine refuses it until it has written its token.
const REFUSED_KEY_GRACE: Duration = Duration::from_secs(8);
/// How long nothing may answer (with no engine this app can start) before
/// it says so.
const NOTHING_GRACE: Duration = Duration::from_secs(10);

/// What the supervisor has told about the engine it can't reach, in a toast
/// that stays until it connects.
fn tell(title: String, body: String) -> String {
    use crate::plugins::host_bar::{Notice, Tone, notice};
    notice(Notice { title: title.clone(), body, tone: Tone::Warning, action: None, stays: true });
    title
}

type Listener = Rc<dyn Fn(&Rpc)>;

pub struct Engine {
    config: Result<Config, manager::Refusal>,
    /// The connection, while there is one.
    rpc: RefCell<Option<Rpc>>,
    /// Its engine's workspace (`local`, `synced`, `development`), once asked.
    scope: RefCell<Option<String>>,
    /// Whether its session is signed in, as the account last heard.
    signed_in: Cell<bool>,
    /// A restart asked of this connection's engine (once each).
    restarting: Cell<bool>,
    listeners: RefCell<Vec<Listener>>,
    /// The latest window's tasks: the connection lives on them, and moves to
    /// the next window's when that window closes.
    tasks: RefCell<Option<TaskSpawner>>,
    supervising: Cell<bool>,
}

thread_local! {
    static ENGINE: Rc<Engine> = Rc::new(Engine::new());
}

/// The engine's data dir, from the config (see `Engine::data_dir`).
pub fn data_dir() -> Option<std::path::PathBuf> {
    get().data_dir()
}

pub fn get() -> Rc<Engine> {
    ENGINE.with(Rc::clone)
}

/// The engine for `cx`'s window: the connection starts with the first
/// window to ask, and moves on if its window closes.
pub fn install(cx: &mut WindowContext) {
    let engine = get();
    *engine.tasks.borrow_mut() = Some(cx.tasks.clone());
    if !engine.supervising.get() {
        engine.supervising.set(true);
        cx.tasks.spawn(engine.clone().supervise());
    }
}

/// Clears `supervising` however the supervisor ends (its window closing
/// drops it), so the next window starts it again.
struct Supervising(Rc<Engine>);

impl Drop for Supervising {
    fn drop(&mut self) {
        self.0.supervising.set(false);
        self.0.rpc.borrow_mut().take();
    }
}

impl Engine {
    fn new() -> Self {
        let account = crate::auth::endpoint();
        let config = Config::from_env(&account.edge, &account.client, account.port);
        match &config {
            Ok(config) => eprintln!("zeron-zgx: engine: {}", config.describe()),
            Err(why) => eprintln!("zeron-zgx: engine: refused ({why}); nothing that needs it works"),
        }
        Self {
            config,
            rpc: RefCell::default(),
            scope: RefCell::default(),
            signed_in: Cell::new(false),
            restarting: Cell::new(false),
            listeners: RefCell::default(),
            tasks: RefCell::default(),
            supervising: Cell::new(false),
        }
    }

    /// Where the engine's IPC is (its port and data folder, which holds its
    /// token): for a helper process of this app's own that talks to it
    /// directly (src/terminal.rs's bridge).
    pub fn ipc_endpoint(&self) -> Option<(u16, std::path::PathBuf)> {
        self.config.as_ref().ok().map(|c| (c.port, c.data_dir.clone()))
    }

    /// How this app reaches and starts its engine (none: refused).
    pub fn config(&self) -> Option<&Config> {
        self.config.as_ref().ok()
    }

    /// Asks the engine to stop: the connection ends and the supervisor
    /// starts the engine on disk in its place (if this app may start one).
    pub async fn stop(&self) -> Result<(), String> {
        let Some(rpc) = self.rpc.borrow().clone() else {
            return Err("offline".into());
        };
        match rpc.call("StopEngine", json!({})).await {
            Ok(_) => Ok(()),
            Err(error) if error == "closed" => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// The connection, while there is one (to start a watch on it now).
    pub fn current(&self) -> Option<Rpc> {
        self.rpc.borrow().clone()
    }

    /// The engine's data dir, as this app's config has it (nothing when the
    /// config refused one).
    pub fn data_dir(&self) -> Option<std::path::PathBuf> {
        self.config.as_ref().ok().map(|c| c.data_dir.clone())
    }

    /// `work` on the connection's tasks.
    pub fn spawn(&self, work: impl std::future::Future<Output = ()> + 'static) {
        if let Some(tasks) = &*self.tasks.borrow() {
            tasks.spawn(work);
        }
    }

    /// `listener` hears every connection from now on (the current one at
    /// once): where it starts its watches.
    pub fn on_connect(&self, listener: impl Fn(&Rpc) + 'static) {
        let listener: Listener = Rc::new(listener);
        self.listeners.borrow_mut().push(listener.clone());
        let current = self.rpc.borrow().clone();
        if let Some(rpc) = current {
            listener(&rpc);
        }
    }

    /// The connection, waiting up to `wait` for one.
    pub async fn connection(&self, wait: Duration) -> Option<Rpc> {
        let until = Instant::now() + wait;
        loop {
            if let Some(rpc) = self.rpc.borrow().clone().filter(|rpc| !rpc.is_closed()) {
                return Some(rpc);
            }
            if self.config.is_err() || Instant::now() >= until {
                return None;
            }
            zgx::timer::sleep(Duration::from_millis(100)).await;
        }
    }

    /// A unary call: its answer, or the engine's error, or `offline` when
    /// there's no engine to ask.
    pub async fn call(&self, method: &str, params: Json) -> Result<Json, String> {
        let Some(rpc) = self.connection(CONNECT_WAIT).await else {
            return Err("offline".into());
        };
        rpc.call(method, params).await
    }

    /// What the account heard of the session: the engine starts again if
    /// its workspace no longer follows it (see [`manager::needs_restart`]).
    pub fn session(&self, signed_in: bool) {
        self.signed_in.set(signed_in);
        self.follow_session();
    }

    fn follow_session(&self) {
        let scope = self.scope.borrow().clone();
        if self.restarting.get() || !manager::needs_restart(scope.as_deref(), self.signed_in.get()) {
            return;
        }
        // (Not an engine this app can't start again: the desktop app's.)
        if !self.config.as_ref().is_ok_and(|c| c.may_restart()) {
            return;
        }
        let Some(rpc) = self.rpc.borrow().clone() else {
            return;
        };
        self.restarting.set(true);
        eprintln!("zeron-zgx: engine: signed in on a local workspace; starting it again to sync");
        self.spawn(async move {
            // It answers, then stops; the connection ends and a new engine
            // is started in its place.
            if let Err(error) = rpc.call("StopEngine", json!({})).await
                && error != "closed"
            {
                eprintln!("zeron-zgx: engine: didn't stop: {error}");
            }
        });
    }

    async fn supervise(self: Rc<Self>) {
        let _supervising = Supervising(self.clone());
        let Ok(config) = &self.config else {
            return;
        };
        let mut spawner = Spawner::new();
        let mut delay = Duration::from_millis(250);
        let mut said = None::<String>;
        // Since when the port refuses this app's key, or nothing answers; and
        // the toast told about it, if any.
        let mut refused_since = None::<Instant>;
        let mut nothing_since = None::<Instant>;
        let mut told = None::<String>;
        loop {
            let token = rpc::read_token(&config.data_dir);
            match rpc::connect(config.port, token).await {
                Ok(rpc) => {
                    delay = Duration::from_millis(250);
                    said = None;
                    (refused_since, nothing_since) = (None, None);
                    if let Some(title) = told.take() {
                        crate::plugins::host_bar::withdraw(&title);
                    }
                    eprintln!("zeron-zgx: engine: connected");
                    // A dev desktop app's engine: which app, to start that
                    // one when nothing answers on a later launch.
                    if config.borrowed()
                        && let Some(exe) = owner::listener_exe(config.port)
                    {
                        manager::remember_engine_app(config, &exe);
                    }
                    *self.rpc.borrow_mut() = Some(rpc.clone());
                    self.scope.borrow_mut().take();
                    self.restarting.set(false);
                    // Which workspace it opened: whether a session change
                    // needs it started again.
                    let (me, info) = (self.clone(), rpc.clone());
                    self.spawn(async move {
                        if let Ok(answer) = info.call("EngineInfo", json!({})).await {
                            *me.scope.borrow_mut() = answer["workspaceScope"].as_str().map(Into::into);
                            me.follow_session();
                        }
                    });
                    let listeners = self.listeners.borrow().clone();
                    for listener in listeners {
                        listener(&rpc);
                    }
                    let reason = rpc.pump().await;
                    self.rpc.borrow_mut().take();
                    eprintln!("zeron-zgx: engine: connection ended ({reason})");
                }
                Err(DialError::Refused) => {
                    refused_since = None;
                    if let Some(title) = told.take_if(|title| title.starts_with("Port ")) {
                        crate::plugins::host_bar::withdraw(&title);
                    }
                    let since = *nothing_since.get_or_insert_with(Instant::now);
                    if spawner.start_if_due(config) {
                        delay = Duration::from_millis(500);
                    } else if !config.starts_engine() && told.is_none() && since.elapsed() >= NOTHING_GRACE {
                        told = Some(tell(
                            format!("No Zeron engine on port {}", config.port),
                            "Open the Zeron app whose engine this uses. Zeron connects by itself once it answers.".into(),
                        ));
                    }
                }
                // A new engine is writing its token: try again shortly. One
                // that keeps refusing it isn't this app's (another data dir:
                // another build's engine holds the port), and is said.
                Err(DialError::Unauthorized) => {
                    nothing_since = None;
                    // (Something answers now: "no engine" no longer holds.)
                    if let Some(title) = told.take_if(|title| title.starts_with("No Zeron engine")) {
                        crate::plugins::host_bar::withdraw(&title);
                    }
                    let since = *refused_since.get_or_insert_with(Instant::now);
                    if told.is_none() && since.elapsed() >= REFUSED_KEY_GRACE {
                        let holder = owner::listener_exe(config.port)
                            .map_or_else(|| "Another program".to_owned(), |exe| exe.display().to_string());
                        told = Some(tell(
                            format!("Port {} is held by another Zeron engine", config.port),
                            format!(
                                "{holder} answers there but doesn't accept this app's key (from {}). Quit that engine (Task Manager → Details) and Zeron connects by itself.",
                                config.data_dir.display()
                            ),
                        ));
                    }
                }
                Err(DialError::Other(error)) => {
                    if said.as_deref() != Some(&error) {
                        eprintln!("zeron-zgx: engine: can't connect: {error}");
                        said = Some(error);
                    }
                }
            }
            zgx::timer::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(3));
        }
    }
}

/// An engine error as a script's outcome word: `offline` (no engine, or the
/// connection dropped mid-call), `signed_out`, or `failed`.
pub fn outcome(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if error == "offline" || error == "closed" || lower.contains("unreachable") {
        "offline"
    } else if lower.contains("signed out") || lower.contains("signed-in") || lower.contains("sign in") {
        "signed_out"
    } else {
        "failed"
    }
}
