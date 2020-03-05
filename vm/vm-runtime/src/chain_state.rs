// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    access_path_helper::AccessPathHelper,
    account::AccountData,
};
use anyhow::{Error, Result};
use types::{
    access_path::AccessPath,
    write_set::{WriteOp, WriteSet},
};
use libra_state_view::StateView;
use libra_types::access_path::AccessPath as LibraAccessPath;
use std::sync::Arc;
use traits::ChainState;
use vm::errors::VMResult;
use vm_runtime::data_cache::{BlockDataCache, RemoteCache};
use logger::prelude::*;

/// Adaptor for chain state
pub struct StateStore<'txn> {
    chain_state: &'txn dyn ChainState,
}

impl<'txn> StateStore<'txn> {
    pub fn new(chain_state: &'txn dyn ChainState) -> Self {
        StateStore { chain_state }
    }

    /// Adds a [`WriteSet`] to state store.
    pub fn add_write_set(&mut self, write_set: &WriteSet) {
        for (access_path, write_op) in write_set {
            match write_op {
                WriteOp::Value(blob) => {
                    self.set(access_path.clone(), blob.clone());
                }
                WriteOp::Deletion => {
                    self.remove(access_path);
                }
            }
        }
    }

    /// Sets a (key, value) pair within state store.
    pub fn set(&mut self, access_path: AccessPath, data_blob: Vec<u8>) -> Result<()> {
        info!("set access_path: {:?}, data_blob: {:?}", access_path, data_blob);
        self.chain_state.set(&access_path, data_blob)
    }

    /// Deletes a key from state store.
    pub fn remove(&mut self, access_path: &AccessPath) -> Result<()> {
        info!("remove access_path: {:?}", access_path);
        self.chain_state.delete(access_path)
    }

    /// Adds an [`AccountData`] to state store.
    pub fn add_account_data(&mut self, account_data: &AccountData) {
        match account_data.to_resource().simple_serialize() {
            Some(blob) => {
                self.set(account_data.make_access_path(), blob);
            }
            None => panic!("can't create Account data"),
        }
    }

}

impl<'txn> StateView for StateStore<'txn> {
    fn get(&self, access_path: &LibraAccessPath) -> Result<Option<Vec<u8>>> {
        ChainState::get(
            self.chain_state,
            &AccessPathHelper::to_Starcoin_AccessPath(access_path),
        )
    }

    fn multi_get(&self, _access_paths: &[LibraAccessPath]) -> Result<Vec<Option<Vec<u8>>>> {
        unimplemented!();
    }

    fn is_genesis(&self) -> bool {
        unimplemented!();
    }
}

// This is used by the `process_transaction` API.
impl<'txn> RemoteCache for StateStore<'txn> {
    fn get(&self, access_path: &LibraAccessPath) -> VMResult<Option<Vec<u8>>> {
        Ok(StateView::get(self, access_path).expect("it should not error"))
    }
}
