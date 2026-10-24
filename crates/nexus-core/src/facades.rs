//! Host-side pieces of a [`nexus_api::PluginContext`].

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use nexus_api::{
    Event, EventSink, HttpClient, HttpRequest, HttpResponse, LogLevel, LogSink, Permissions, PluginError, PluginId,
    PluginResult,
};

use crate::{EventBus, HttpTransport};

pub(crate) struct TracingLog {
    pub plugin: PluginId,
}

impl LogSink for TracingLog {
    fn log(&self, level: LogLevel, message: &str) {
        let plugin_id = self.plugin.as_str();
        match level {
            LogLevel::Trace => tracing::trace!(plugin_id, "{message}"),
            LogLevel::Debug => tracing::debug!(plugin_id, "{message}"),
            LogLevel::Info => tracing::info!(plugin_id, "{message}"),
            LogLevel::Warn => tracing::warn!(plugin_id, "{message}"),
            LogLevel::Error => tracing::error!(plugin_id, "{message}"),
        }
    }
}

pub(crate) struct PluginEvents {
    pub plugin: PluginId,
    pub bus: EventBus,
}

impl EventSink for PluginEvents {
    fn publish(&self, topic: &str, payload: serde_json::Value) -> PluginResult<()> {
        self.bus.publish(Event::Custom { plugin: self.plugin.clone(), topic: topic.to_owned(), payload });
        Ok(())
    }

    fn subscribe(&self) -> PluginResult<BoxStream<'static, Event>> {
        Ok(self.bus.stream())
    }
}

/// The host part of a URL, if the URL is `http` or `https` and has no user info.
pub(crate) fn url_host(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    let web = matches!(url.scheme(), "http" | "https");
    let no_user_info = url.username().is_empty() && url.password().is_none();
    (web && no_user_info).then(|| url.host_str().map(str::to_owned)).flatten()
}

pub(crate) fn check_url(plugin: &PluginId, permissions: &Permissions, url: &str) -> PluginResult<()> {
    match url_host(url) {
        Some(host) if permissions.allows_host(&host) => Ok(()),
        Some(host) => Err(PluginError::PermissionDenied(format!("{plugin} may not reach {host}"))),
        None => Err(PluginError::PermissionDenied(format!("{plugin}: {url} is not an allowed web address"))),
    }
}

/// The most redirects a request follows.
pub(crate) const MAX_REDIRECTS: usize = 10;

/// `location` (maybe relative) against the URL that sent it, checked against the allowlist.
pub(crate) fn next_hop(
    plugin: &PluginId,
    permissions: &Permissions,
    from: &str,
    location: &str,
) -> PluginResult<String> {
    let base = url::Url::parse(from).map_err(|e| PluginError::Network(format!("{from}: {e}")))?;
    let next = base.join(location).map_err(|e| PluginError::Network(format!("bad redirect {location}: {e}")))?;
    let next = next.to_string();
    check_url(plugin, permissions, &next)?;
    Ok(next)
}

/// Sends `request` and follows redirects, checking every hop against the allowlist.
pub(crate) async fn send_checked(
    transport: &dyn HttpTransport,
    plugin: &PluginId,
    permissions: &Permissions,
    mut request: HttpRequest,
) -> PluginResult<HttpResponse> {
    check_url(plugin, permissions, &request.url)?;
    for _ in 0..=MAX_REDIRECTS {
        let response = transport.send(request.clone()).await?;
        let location = response.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("location")).map(|(_, v)| v);
        let (true, Some(location)) = ((300..400).contains(&response.status), location) else { return Ok(response) };
        request.url = next_hop(plugin, permissions, &request.url, location)?;
        // 303, and 301 or 302 after a POST, go on as a GET without the body.
        if response.status == 303 || (request.method == nexus_api::Method::Post && response.status <= 302) {
            request.method = nexus_api::Method::Get;
            request.body = None;
        }
    }
    Err(PluginError::Network(format!("more than {MAX_REDIRECTS} redirects")))
}

/// HTTP limited to the manifest's network allowlist.
pub(crate) struct AllowlistHttp {
    pub plugin: PluginId,
    pub permissions: Permissions,
    pub transport: Arc<dyn HttpTransport>,
}

#[async_trait]
impl HttpClient for AllowlistHttp {
    async fn send(&self, request: HttpRequest) -> PluginResult<HttpResponse> {
        send_checked(self.transport.as_ref(), &self.plugin, &self.permissions, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_info_and_other_schemes_are_not_web_addresses() {
        assert_eq!(url_host("https://github.com/x").as_deref(), Some("github.com"));
        assert_eq!(url_host("https://github.com@evil.net/x"), None);
        assert_eq!(url_host("file:///etc/passwd"), None);
        assert_eq!(url_host("not a url"), None);
    }
}
