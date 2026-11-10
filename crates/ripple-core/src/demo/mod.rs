//! Placeholder; replaced by the scripted demo backend.
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::api::{NetOp, Network};
use crate::app::App;

pub struct DemoNet;

impl DemoNet {
    pub fn new() -> Self {
        DemoNet
    }
}

#[async_trait]
impl Network for DemoNet {
    async fn start(&self, _app: Arc<App>) -> Result<()> {
        Ok(())
    }
    async fn exec(&self, _op: NetOp) -> Result<Value> {
        Ok(Value::Null)
    }
    fn is_live(&self) -> bool {
        false
    }
}
