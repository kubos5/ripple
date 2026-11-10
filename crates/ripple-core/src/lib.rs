//! Ripple's core: everything except drawing the interface.
//!
//! The UI talks to [`Ripple`] with [`api::Request`]s and listens for
//! [`api::UiEvent`]s. Behind it sits an [`app::App`] bound to a network
//! backend: the real WhatsApp service, or a self-contained demo.

pub mod api;
pub mod app;
pub mod demo;
pub mod link;
pub mod media;
pub mod model;
pub mod store;
#[cfg(feature = "voice")]
pub mod voice;
#[cfg(feature = "live")]
pub mod wa;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use serde_json::Value;
use tokio::sync::broadcast;

use api::{Network, Request, UiEvent};
use app::App;

/// Owns the active [`App`] and swaps it when the user enters or leaves the demo.
pub struct Ripple {
    data_dir: PathBuf,
    events: broadcast::Sender<UiEvent>,
    app: RwLock<Arc<App>>,
}

impl Ripple {
    /// Opens the data directory. With `demo` set, starts straight in the demo.
    pub async fn open(data_dir: PathBuf, demo: bool) -> Result<Arc<Ripple>> {
        let (events, _) = broadcast::channel(8192);
        let app = Self::build(&data_dir, &events, demo).await?;
        Ok(Arc::new(Ripple { data_dir, events, app: RwLock::new(app) }))
    }

    async fn build(data_dir: &std::path::Path, events: &broadcast::Sender<UiEvent>, demo: bool) -> Result<Arc<App>> {
        if demo {
            let root = data_dir.join("demo");
            // The demo always starts from the same scripted state.
            let _ = std::fs::remove_dir_all(&root);
            let app = App::new(root, events.clone(), true)?;
            app.start(Arc::new(demo::DemoNet::new())).await?;
            return Ok(app);
        }
        let app = App::new(data_dir.join("live"), events.clone(), false)?;
        app.start(Self::live_net()).await?;
        Ok(app)
    }

    #[cfg(feature = "live")]
    fn live_net() -> Arc<dyn Network> {
        Arc::new(wa::LiveNet::new())
    }

    #[cfg(not(feature = "live"))]
    fn live_net() -> Arc<dyn Network> {
        Arc::new(api::NoNetwork)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<UiEvent> {
        self.events.subscribe()
    }

    pub fn app(&self) -> Arc<App> {
        self.app.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    async fn switch(&self, demo: bool) -> Result<()> {
        let old = self.app();
        if old.demo == demo {
            return Ok(());
        }
        old.net().shutdown().await;
        let app = Self::build(&self.data_dir, &self.events, demo).await?;
        *self.app.write().unwrap_or_else(|e| e.into_inner()) = app;
        let _ = self.events.send(UiEvent::Reset);
        Ok(())
    }

    pub async fn handle(&self, req: Request) -> Result<Value> {
        match req {
            Request::DemoStart => self.switch(true).await.map(|_| Value::Null),
            Request::DemoExit => self.switch(false).await.map(|_| Value::Null),
            other => self.app().handle(other).await,
        }
    }

    /// Resolves a media token for the shell's file-serving protocol.
    pub fn media_path(&self, token: &str) -> Option<PathBuf> {
        self.app().media.path(token).filter(|p| p.is_file())
    }
}
