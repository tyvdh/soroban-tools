use std::str::FromStr;

use clap::{arg, Parser};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stellar_strkey::ed25519::PublicKey;

use crate::{
    commands::HEADING_RPC,
    rpc::{self, Client},
};

use super::locator;

pub mod add;
pub mod ls;
pub mod rm;

#[derive(Debug, Parser)]
pub enum Cmd {
    /// Add a new network
    Add(add::Cmd),
    /// Remove a network
    Rm(rm::Cmd),
    /// List networks
    Ls(ls::Cmd),
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Add(#[from] add::Error),

    #[error(transparent)]
    Rm(#[from] rm::Error),

    #[error(transparent)]
    Ls(#[from] ls::Error),

    #[error(transparent)]
    Config(#[from] locator::Error),

    #[error("network arg or rpc url  and network passphrase are required if using the network")]
    Network,
    #[error(transparent)]
    Rpc(#[from] rpc::Error),
    #[error(transparent)]
    Hyper(#[from] hyper::Error),
    #[error("Failed to parse JSON from {0}, {1}")]
    FailedToParseJSON(String, serde_json::Error),
    #[error("Invalid URL {0}")]
    InvalidUrl(String),
    #[error("Inproper response {0}")]
    InproperResponse(String),
    #[error("Currently not supported on windows. Please visit:\n{0}")]
    WindowsNotSupported(String),
}

impl Cmd {
    pub fn run(&self) -> Result<(), Error> {
        match self {
            Cmd::Add(cmd) => cmd.run()?,
            Cmd::Rm(new) => new.run()?,
            Cmd::Ls(cmd) => cmd.run()?,
        };
        Ok(())
    }
}

#[derive(Debug, clap::Args, Clone, Default)]
#[group(skip)]
pub struct Args {
    /// RPC server endpoint
    #[arg(
        long = "rpc-url",
        requires = "network_passphrase",
        env = "SOROBAN_RPC_URL",
        help_heading = HEADING_RPC,
    )]
    pub rpc_url: Option<String>,
    /// Network passphrase to sign the transaction sent to the rpc server
    #[arg(
        long = "network-passphrase",
        requires = "rpc_url",
        env = "SOROBAN_NETWORK_PASSPHRASE",
        help_heading = HEADING_RPC,
    )]
    pub network_passphrase: Option<String>,
    /// Name of network to use from config
    #[arg(
        long,
        conflicts_with = "network_passphrase",
        conflicts_with = "rpc_url",
        env = "SOROBAN_NETWORK",
        help_heading = HEADING_RPC,
    )]
    pub network: Option<String>,
}

impl Args {
    pub fn get(&self, locator: &locator::Args) -> Result<Network, Error> {
        if let Some(name) = self.network.as_deref() {
            Ok(locator.read_network(name)?)
        } else if let (Some(rpc_url), Some(network_passphrase)) =
            (self.rpc_url.clone(), self.network_passphrase.clone())
        {
            Ok(Network {
                rpc_url,
                network_passphrase,
            })
        } else {
            Err(Error::Network)
        }
    }

    pub fn is_no_network(&self) -> bool {
        self.network.is_none() && self.network_passphrase.is_none() && self.rpc_url.is_none()
    }
}

#[derive(Debug, clap::Args, Serialize, Deserialize, Clone)]
#[group(skip)]
pub struct Network {
    /// RPC server endpoint
    #[arg(
        long = "rpc-url",
        env = "SOROBAN_RPC_URL",
        help_heading = HEADING_RPC,
    )]
    pub rpc_url: String,
    /// Network passphrase to sign the transaction sent to the rpc server
    #[arg(
            long,
            env = "SOROBAN_NETWORK_PASSPHRASE",
            help_heading = HEADING_RPC,
        )]
    pub network_passphrase: String,
}

impl Network {
    pub async fn helper_url(&self, addr: &str) -> Result<http::Uri, Error> {
        tracing::debug!("address {addr:?}");
        let client = Client::new(&self.rpc_url)?;
        let helper_url_root = client.friendbot_url().await?;
        let uri = http::Uri::from_str(&helper_url_root)
            .map_err(|_| Error::InvalidUrl(helper_url_root.to_string()))?;
        http::Uri::from_str(&format!("{uri:?}?addr={addr}"))
            .map_err(|_| Error::InvalidUrl(helper_url_root.to_string()))
    }

    #[allow(clippy::similar_names)]
    pub async fn fund_address(&self, addr: &PublicKey) -> Result<(), Error> {
        let uri = self.helper_url(&addr.to_string()).await?;
        tracing::debug!("URL {uri:?}");
        let response = match uri.scheme_str() {
            Some("http") => hyper::Client::new().get(uri.clone()).await?,
            Some("https") => {
                #[cfg(target_os = "windows")]
                {
                    return Err(Error::WindowsNotSupported(uri.to_string()));
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let https = hyper_tls::HttpsConnector::new();
                    hyper::Client::builder()
                        .build::<_, hyper::Body>(https)
                        .get(uri.clone())
                        .await?
                }
            }
            _ => {
                return Err(Error::InvalidUrl(uri.to_string()));
            }
        };
        let body = hyper::body::to_bytes(response.into_body()).await?;
        let res = serde_json::from_slice::<serde_json::Value>(&body)
            .map_err(|e| Error::FailedToParseJSON(uri.to_string(), e))?;
        tracing::debug!("{res:#?}");
        if let Some(detail) = res.get("detail").and_then(Value::as_str) {
            if detail.contains("createAccountAlreadyExist") {
                tracing::warn!("Account already exists");
            }
        } else if res.get("successful").is_none() {
            return Err(Error::InproperResponse(res.to_string()));
        }
        Ok(())
    }
}

impl Network {
    pub fn futurenet() -> Self {
        Network {
            rpc_url: "https://rpc-futurenet.stellar.org:443".to_owned(),
            network_passphrase: "Test SDF Future Network ; October 2022".to_owned(),
        }
    }
}
