use std::{fmt::Debug, path::PathBuf};

use clap::{command, Parser};
use soroban_spec_typescript::{self as typescript, boilerplate::Project};

use crate::wasm;
use crate::{
    commands::{
        config::{
            ledger_file, locator,
            network::{self, Network},
        },
        contract::{self, fetch},
    },
    utils::contract_spec::{self, ContractSpec},
};

#[derive(Parser, Debug, Clone)]
#[group(skip)]
pub struct Cmd {
    /// Path to optional wasm binary
    #[arg(long)]
    pub wasm: Option<std::path::PathBuf>,

    /// where to place generated project
    #[arg(long)]
    output_dir: PathBuf,

    #[arg(long)]
    contract_name: String,

    #[arg(long, alias = "id")]
    contract_id: String,

    #[command(flatten)]
    locator: locator::Args,

    #[command(flatten)]
    network: network::Args,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("failed generate TS from file: {0}")]
    GenerateTSFromFile(typescript::GenerateFromFileError),
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("--root-dir cannot be a file: {0:?}")]
    IsFile(PathBuf),

    #[error(transparent)]
    Network(#[from] network::Error),

    #[error(transparent)]
    Locator(#[from] locator::Error),
    #[error(transparent)]
    Fetch(#[from] fetch::Error),
    #[error(transparent)]
    Spec(#[from] contract_spec::Error),
}

impl Cmd {
    pub async fn run(&self) -> Result<(), Error> {
        let spec = if let Some(wasm) = &self.wasm {
            let wasm: wasm::Args = wasm.into();
            wasm.parse().unwrap().spec
        } else {
            let fetch = contract::fetch::Cmd {
                contract_id: self.contract_id.clone(),
                out_file: None,
                locator: self.locator.clone(),
                network: self.network.clone(),
                ledger_file: ledger_file::Args::default(),
            };
            let bytes = fetch.get_bytes().await?;
            ContractSpec::new(&bytes)?.spec
        };
        if self.output_dir.is_file() {
            return Err(Error::IsFile(self.output_dir.clone()));
        }
        let output_dir = if self.output_dir.exists() {
            self.output_dir.join(&self.contract_name)
        } else {
            self.output_dir.clone()
        };
        std::fs::create_dir_all(&output_dir)?;
        let p: Project = output_dir.clone().try_into()?;
        let Network {
            rpc_url,
            network_passphrase,
            ..
        } = self
            .network
            .get(&self.locator)
            .ok()
            .unwrap_or_else(Network::futurenet);
        p.init(
            &self.contract_name,
            &self.contract_id,
            &rpc_url,
            &network_passphrase,
            &spec,
        )?;
        std::process::Command::new("npm")
            .arg("install")
            .current_dir(&output_dir)
            .spawn()?
            .wait()?;
        std::process::Command::new("npm")
            .arg("run")
            .arg("build")
            .current_dir(&output_dir)
            .spawn()?
            .wait()?;
        Ok(())
    }
}
