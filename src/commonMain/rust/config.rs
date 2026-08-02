use iroh::endpoint::presets::{self, Preset as _};
use iroh::endpoint::Builder;
use iroh::{RelayMode, RelayUrl, SecretKey};

use crate::error::IrohError;

/// Relay configuration for [`EndpointOptions`].
#[derive(uniffi::Enum, Default)]
pub enum RelayModeOption {
    /// The n0 production relays (what [`crate::IrohEndpoint::bind`] uses).
    #[default]
    Default,
    /// No relays; only direct connections and whatever addresses the address lookup
    /// service or caller provides.
    Disabled,
    /// A caller-supplied set of relay server URLs.
    Custom { urls: Vec<String> },
}

/// Options for [`crate::IrohEndpoint::bind_with`]. Left at their defaults, these reproduce
/// [`crate::IrohEndpoint::bind`] exactly.
#[derive(uniffi::Record)]
pub struct EndpointOptions {
    pub alpns: Vec<String>,
    /// 32 raw bytes; a fresh key is generated when omitted.
    #[uniffi(default = None)]
    pub secret_key: Option<Vec<u8>>,
    /// `None` means [`RelayModeOption::Default`] — UniFFI 0.29 record defaults only accept
    /// literals/`None`/`Some`/`[]`, so a non-literal enum variant can't be the default value
    /// directly.
    #[uniffi(default = None)]
    pub relay_mode: Option<RelayModeOption>,
    /// Whether to publish and resolve addresses via n0's address lookup services — whichever
    /// set `presets::N0` currently installs (as of iroh 1.0.3: pkarr publish, plus pkarr-over-
    /// HTTPS and DNS resolution). `false` removes all of them, leaving an endpoint that can
    /// only reach peers whose direct addresses or relay URL the caller supplies.
    #[uniffi(default = true)]
    pub address_lookup: bool,
    /// Explicit local socket address to bind, e.g. `"0.0.0.0:11204"`.
    #[uniffi(default = None)]
    pub bind_addr: Option<String>,
    #[uniffi(default = None)]
    pub external_addrs: Option<Vec<String>>,
    /// Whether to spawn the background `online()` warm-up, as `bind` always does.
    #[uniffi(default = true)]
    pub warm_up_online: bool,
}

/// Turns [`EndpointOptions`] into a ready-to-[`Builder::bind`] builder. Shared by
/// [`crate::IrohEndpoint::bind`] (via a fixed [`EndpointOptions`]) and
/// [`crate::IrohEndpoint::bind_with`].
pub(crate) fn builder_from_options(options: EndpointOptions) -> Result<Builder, IrohError> {
    let EndpointOptions {
        alpns,
        secret_key,
        relay_mode,
        address_lookup,
        bind_addr,
        external_addrs,
        warm_up_online: _,
    } = options;
    let relay_mode = relay_mode.unwrap_or_default();

    // Every configuration starts from `presets::N0` and subtracts, rather than composing the
    // non-default cases up from `presets::Minimal`. Hand-listing the preset's contents is how
    // this wrapper silently fell behind before: iroh 1.0.3 added `PkarrResolver::n0_dns()` to
    // N0 (1.0.0 had it only under `cfg(wasm_browser)`), and the hand-built branch kept
    // publishing via pkarr while resolving over DNS alone. Starting from the preset means the
    // next service n0 adds arrives here for free.
    //
    // The subtractions are exact, not approximate: `relay_mode` *replaces* the preset's relay
    // transport (and `Disabled` removes it outright), and `clear_address_lookup` drops every
    // lookup the preset registered — leaving just the crypto provider, which is all
    // `presets::Minimal` ever contributed. So each case below is byte-for-byte what the old
    // code produced, except for the resolver it was missing.
    let mut builder = presets::N0.apply(Builder::empty());

    builder = match relay_mode {
        // Already applied by the preset.
        RelayModeOption::Default => builder,
        RelayModeOption::Disabled => builder.relay_mode(RelayMode::Disabled),
        RelayModeOption::Custom { urls } => {
            let relay_urls = urls
                .iter()
                .map(|u| u.parse::<RelayUrl>().map_err(IrohError::msg))
                .collect::<Result<Vec<_>, _>>()?;
            builder.relay_mode(RelayMode::custom(relay_urls))
        }
    };

    if !address_lookup {
        builder = builder.clear_address_lookup();
    }

    if let Some(bytes) = secret_key {
        let key: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| IrohError::Generic { msg: "secret key must be 32 bytes".into() })?;
        builder = builder.secret_key(SecretKey::from_bytes(&key));
    }

    let alpns: Vec<Vec<u8>> = alpns.into_iter().map(String::into_bytes).collect();
    builder = builder.alpns(alpns);

    if let Some(addr) = bind_addr {
        builder = builder.bind_addr(addr.as_str()).map_err(IrohError::msg)?;
    }

    if let Some(addrs) = external_addrs {
        for addr in addrs {
            let sock: std::net::SocketAddr = addr.parse().map_err(IrohError::msg)?;
            builder = builder.external_addr(sock);
        }
    }

    Ok(builder)
}
