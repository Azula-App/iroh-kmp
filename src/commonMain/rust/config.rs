use iroh::address_lookup::{DnsAddressLookup, PkarrPublisher};
use iroh::endpoint::presets::{self, Preset as _};
use iroh::endpoint::{default_relay_mode, Builder};
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
    /// Whether to publish/resolve addresses via n0's DNS address lookup service.
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

    // The common case mirrors `presets::N0.apply(Builder::empty())` exactly, so `bind`'s
    // delegation to `bind_with` produces a byte-for-byte identical endpoint. Anything else
    // is composed manually from `presets::Minimal` (just the crypto provider).
    let mut builder = if matches!(relay_mode, RelayModeOption::Default) && address_lookup {
        presets::N0.apply(Builder::empty())
    } else {
        let mut b = presets::Minimal.apply(Builder::empty());
        b = match relay_mode {
            RelayModeOption::Default => b.relay_mode(default_relay_mode()),
            RelayModeOption::Disabled => b.relay_mode(RelayMode::Disabled),
            RelayModeOption::Custom { urls } => {
                let relay_urls = urls
                    .iter()
                    .map(|u| u.parse::<RelayUrl>().map_err(IrohError::msg))
                    .collect::<Result<Vec<_>, _>>()?;
                b.relay_mode(RelayMode::custom(relay_urls))
            }
        };
        if address_lookup {
            b = b
                .address_lookup(PkarrPublisher::n0_dns())
                .address_lookup(DnsAddressLookup::n0_dns());
        } else {
            b = b.clear_address_lookup();
        }
        b
    };

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
