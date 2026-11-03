//! How a client's `base_url` reads, for the report.
//!
//! [`crate::usages`] routes a client given a `base_url` by what the value
//! resolves to: the provider's own host (or `None`, the SDK default) stays
//! direct, another host is [`Routing::CustomBaseUrl`], and an environment
//! lookup or a value that does not resolve leaves the routing unknown or
//! direct with an `ambiguous_client` gap at each call (`base_url_env`,
//! `base_url_unresolved`). [`super::check`] follows that routing, treats the
//! gap like any other client gap (the routing is unknown until confirmed)
//! and only names the class here, so `arbor usages` and `arbor retirements`
//! always agree. The URL itself is never reported.

use crate::usages::{ClientConstruction, GapKind, Routing, SdkCall, UsageGap};
use serde::Serialize;

/// How a client's `base_url` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseUrl {
    /// The provider's own API host (`https://api.openai.com/v1`), directly
    /// or through a same-file constant, or `None` (the SDK default): a
    /// direct call.
    ProviderHost,
    /// An environment lookup whose fallback is the provider's host
    /// (`os.getenv("OPENAI_BASE_URL", "https://api.openai.com/v1")`), what
    /// the SDK does when no `base_url` is given. Direct unless the
    /// deployment sets the variable, so the routing is unknown until
    /// confirmed, as the usage scan's `base_url_env` gap says.
    EnvProviderDefault,
    /// An environment lookup with no fallback (`os.environ.get("BASE_URL")`):
    /// the host depends on the deployment, so the routing is unknown.
    EnvOnly,
    /// Another host (a literal, a constant or an environment fallback):
    /// OpenRouter, LiteLLM, a local server, a proxy.
    OtherHost,
    /// A value that does not resolve to a string in the file (a function
    /// call, an attribute, an f-string): the routing is unknown.
    Unreadable,
}

/// The constructor options that set a client's endpoint: Python's
/// `base_url`/`api_base`, the JavaScript SDKs' `baseURL` and openai v3's
/// `basePath`.
fn is_base_url_key(key: &str) -> bool {
    matches!(key, "base_url" | "api_base" | "baseURL" | "basePath")
}

/// The class of the `base_url` behind `call`, or `None` when its client was
/// given none. `at_call` are the usage scan's gaps at the call.
pub(super) fn base_url_of(
    call: &SdkCall,
    at_call: &[&UsageGap],
    clients: &[ClientConstruction],
) -> Option<BaseUrl> {
    if call.routing == Routing::CustomBaseUrl {
        return Some(BaseUrl::OtherHost);
    }
    let gap = at_call.iter().find(|g| {
        g.kind == GapKind::AmbiguousClient
            && matches!(g.reason.as_str(), "base_url_env" | "base_url_unresolved")
    });
    let direct = matches!(
        call.routing,
        Routing::OpenaiDirect | Routing::AnthropicDirect
    );
    match gap.map(|g| g.reason.as_str()) {
        Some("base_url_env") if direct => Some(BaseUrl::EnvProviderDefault),
        Some("base_url_env") => Some(BaseUrl::EnvOnly),
        Some(_) => Some(BaseUrl::Unreadable),
        None if direct => {
            let line = call.client_line?;
            clients
                .iter()
                .any(|c| {
                    c.file == call.file
                        && c.span.line == line
                        && c.kwargs.iter().any(|k| is_base_url_key(k))
                })
                .then_some(BaseUrl::ProviderHost)
        }
        None => None,
    }
}
