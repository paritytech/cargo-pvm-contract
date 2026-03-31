use std::{marker::PhantomData, path::PathBuf};
use subxt::{Config, tx};
use url::Url;

use crate::url_to_string;

/// Arguments required for creating and sending an extrinsic to a Substrate node.
#[derive(Clone)]
pub struct ExtrinsicOpts<C: Config, Signer: Clone> {
    file: Option<PathBuf>,
    manifest_path: Option<PathBuf>,
    url: url::Url,
    signer: Signer,
    storage_deposit_limit: Option<u128>,
    _marker: PhantomData<C>,
}

pub struct ExtrinsicOptsBuilder<C: Config, Signer: Clone> {
    opts: ExtrinsicOpts<C, Signer>,
}

impl<C: Config, Signer> ExtrinsicOptsBuilder<C, Signer>
where
    Signer: tx::Signer<C> + Clone,
{
    /// Returns a clean builder for [`ExtrinsicOpts`].
    pub fn new(signer: Signer) -> ExtrinsicOptsBuilder<C, Signer> {
        ExtrinsicOptsBuilder {
            opts: ExtrinsicOpts {
                file: None,
                manifest_path: None,
                url: url::Url::parse("ws://localhost:9944").unwrap(),
                signer,
                storage_deposit_limit: None,
                _marker: PhantomData,
            },
        }
    }

    /// Sets the path to the contract build artifact file.
    pub fn file<T: Into<PathBuf>>(mut self, file: Option<T>) -> Self {
        self.opts.file = file.map(|f| f.into());
        self
    }

    /// Sets the path to the Cargo.toml of the contract.
    pub fn manifest_path<T: Into<PathBuf>>(mut self, manifest_path: Option<T>) -> Self {
        self.opts.manifest_path = manifest_path.map(|f| f.into());
        self
    }

    /// Sets the websockets url of a Substrate node.
    pub fn url<T: Into<Url>>(mut self, url: T) -> Self {
        self.opts.url = url.into();
        self
    }

    /// Sets the maximum amount of balance that can be charged from the caller to pay
    /// for storage.
    pub fn storage_deposit_limit(mut self, storage_deposit_limit: Option<u128>) -> Self {
        self.opts.storage_deposit_limit = storage_deposit_limit;
        self
    }

    pub fn done(self) -> ExtrinsicOpts<C, Signer> {
        self.opts
    }
}

impl<C: Config, Signer> ExtrinsicOpts<C, Signer>
where
    Signer: tx::Signer<C> + Clone,
{
    /// Sets a new storage deposit limit.
    pub fn set_storage_deposit_limit(&mut self, limit: Option<u128>) {
        self.storage_deposit_limit = limit;
    }

    /// Return the file path of the contract artifact.
    pub fn file(&self) -> Option<&PathBuf> {
        self.file.as_ref()
    }

    /// Return the path to the `Cargo.toml` of the contract.
    pub fn manifest_path(&self) -> Option<&PathBuf> {
        self.manifest_path.as_ref()
    }

    /// Return the URL of the Substrate node.
    pub fn url(&self) -> String {
        url_to_string(&self.url)
    }

    /// Return the signer.
    pub fn signer(&self) -> &Signer {
        &self.signer
    }

    /// Return the storage deposit limit.
    pub fn storage_deposit_limit(&self) -> Option<u128> {
        self.storage_deposit_limit
    }
}
