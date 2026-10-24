//! Tower-style middleware. The host builds the chain and orders it by [`Order`].

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{GameKey, LaunchSpec, Order, PluginResult};

/// The end of a chain: what runs when every middleware calls `next`.
#[async_trait]
pub trait Endpoint<Req: Send + 'static, Resp: Send + 'static>: Send + Sync {
    async fn call(&self, request: Req) -> PluginResult<Resp>;
}

/// One link in a chain. It can change the request, call `next`, change the response, or stop
/// the chain by returning without calling `next`.
#[async_trait]
pub trait Middleware<Req: Send + 'static, Resp: Send + 'static>: Send + Sync {
    /// Unique among the chain's links; [`Order`] refers to it.
    fn name(&self) -> &str;

    fn order(&self) -> Order {
        Order::default()
    }

    async fn call(&self, request: Req, next: Next<'_, Req, Resp>) -> PluginResult<Resp>;
}

/// The rest of the chain.
pub struct Next<'a, Req, Resp> {
    rest: &'a [Arc<dyn Middleware<Req, Resp>>],
    endpoint: &'a dyn Endpoint<Req, Resp>,
}

impl<'a, Req: Send + 'static, Resp: Send + 'static> Next<'a, Req, Resp> {
    /// `chain` must already be in order.
    pub fn new(chain: &'a [Arc<dyn Middleware<Req, Resp>>], endpoint: &'a dyn Endpoint<Req, Resp>) -> Self {
        Self { rest: chain, endpoint }
    }

    pub async fn run(self, request: Req) -> PluginResult<Resp> {
        match self.rest.split_first() {
            Some((first, rest)) => first.call(request, Next { rest, endpoint: self.endpoint }).await,
            None => self.endpoint.call(request).await,
        }
    }
}

/// What launch middleware sees: the game and the spec its emulator plugin prepared.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchRequest {
    pub game: GameKey,
    pub spec: LaunchSpec,
}

/// Runs before the host spawns any game. It can add environment variables or arguments, or
/// stop the launch with an error (for example `Precondition("firmware missing")`).
pub type LaunchMiddleware = dyn Middleware<LaunchRequest, LaunchSpec>;
