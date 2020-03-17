// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::chain_state::StateStore;
use anyhow::Result;
use config::VMConfig;
use crypto::ed25519::compat;

use once_cell::sync::Lazy;

use std::convert::TryInto;

use traits::ChainState;
use types::{
    access_path::AccessPath,
    account_address::AccountAddress,
    account_config::AccountResource,
    transaction::{
        RawUserTransaction, Script, SignedUserTransaction, Transaction, TransactionArgument,
        TransactionOutput, TransactionPayload, TransactionStatus,
    },
    vm_error::{StatusCode, VMStatus},
};

enum MockTransaction {
    Mint {
        sender: AccountAddress,
        amount: u64,
    },
    Payment {
        sender: AccountAddress,
        recipient: AccountAddress,
        amount: u64,
    },
}

pub static KEEP_STATUS: Lazy<TransactionStatus> =
    Lazy::new(|| TransactionStatus::Keep(VMStatus::new(StatusCode::EXECUTED)));

// We use 10 as the assertion error code for insufficient balance within the Libra coin contract.
pub static DISCARD_STATUS: Lazy<TransactionStatus> = Lazy::new(|| {
    TransactionStatus::Discard(VMStatus::new(StatusCode::ABORTED).with_sub_status(10))
});

#[derive(Clone)]
pub struct MockVM {
    config: VMConfig,
}

impl MockVM {
    pub fn new(config: &VMConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    pub fn create_account(
        &self,
        account_address: AccountAddress,
        chain_state: &dyn ChainState,
    ) -> Result<()> {
        let state_store = StateStore::new(chain_state);
        state_store.create_account(account_address)
    }

    pub fn execute_transaction(
        &mut self,
        chain_state: &dyn ChainState,
        txn: Transaction,
    ) -> Result<TransactionOutput> {
        let mut state_store = StateStore::new(chain_state);
        let output;

        match txn {
            Transaction::UserTransaction(txn) => match decode_transaction(&txn) {
                MockTransaction::Mint { sender, amount } => {
                    let access_path = AccessPath::new_for_account(sender);
                    let account_resource: AccountResource = state_store
                        .get_from_statedb(&access_path)?
                        .unwrap()
                        .try_into()?;
                    let new_account_resource = AccountResource::new(
                        amount,
                        1,
                        account_resource.authentication_key().clone(),
                    );
                    state_store
                        .set(access_path, new_account_resource.try_into()?)
                        .unwrap();
                    output = TransactionOutput::new(vec![], 0, KEEP_STATUS.clone());
                }
                MockTransaction::Payment {
                    sender,
                    recipient,
                    amount,
                } => {
                    let access_path_sender = AccessPath::new_for_account(sender);
                    let access_path_receiver = AccessPath::new_for_account(recipient);

                    let account_resource_sender: AccountResource = state_store
                        .get_from_statedb(&access_path_sender)?
                        .expect("txn sender must exist.")
                        .try_into()?;
                    let account_resource_receiver: AccountResource = state_store
                        .get_from_statedb(&access_path_receiver)
                        .and_then(|blob| match blob {
                            Some(blob) => Ok(blob),
                            None => {
                                state_store.create_account(recipient)?;
                                Ok(state_store
                                    .get_from_statedb(&access_path_receiver)?
                                    .expect("account resource must exist."))
                            }
                        })
                        .and_then(|blob| blob.try_into())?;

                    let balance_sender = account_resource_sender.balance();
                    let balance_receiver = account_resource_receiver.balance();
                    let deduction;

                    if balance_sender < amount {
                        deduction = balance_sender;
                    } else {
                        deduction = amount;
                    }

                    let new_account_resource_sender = AccountResource::new(
                        balance_sender - deduction,
                        account_resource_sender.sequence_number() + 1,
                        account_resource_sender.authentication_key().clone(),
                    );
                    let new_account_resource_receiver = AccountResource::new(
                        balance_receiver + deduction,
                        account_resource_sender.sequence_number(),
                        account_resource_receiver.authentication_key().clone(),
                    );
                    state_store.set(access_path_sender, new_account_resource_sender.try_into()?)?;
                    state_store.set(
                        access_path_receiver,
                        new_account_resource_receiver.try_into()?,
                    )?;
                    output = TransactionOutput::new(
                        vec![],
                        0,
                        TransactionStatus::Keep(VMStatus::new(StatusCode::EXECUTED)),
                    );
                }
            },
            Transaction::BlockMetadata(block_metadata) => {
                let (_id, _timestamp, author) = block_metadata.into_inner().unwrap();
                let access_path = AccessPath::new_for_account(author);
                let account_resource: AccountResource = state_store
                    .get_from_statedb(&access_path)
                    .and_then(|blob| match blob {
                        Some(blob) => Ok(blob),
                        None => {
                            state_store.create_account(author)?;
                            Ok(state_store
                                .get_from_statedb(&access_path)?
                                .expect("account resource must exist."))
                        }
                    })
                    .and_then(|blob| blob.try_into())?;

                let new_account_resource = AccountResource::new(
                    account_resource.balance() + 50_00000000,
                    account_resource.sequence_number(),
                    account_resource.authentication_key().clone(),
                );
                state_store.set(access_path, new_account_resource.try_into()?)?;
                output = TransactionOutput::new(vec![], 0, KEEP_STATUS.clone());
            }
            Transaction::StateSet(state_set) => {
                let result_status = match chain_state.apply(state_set) {
                    Ok(_) => KEEP_STATUS.clone(),
                    Err(_) => DISCARD_STATUS.clone(),
                };
                output = TransactionOutput::new(vec![], 0, result_status)
            }
        }
        Ok(output)
    }
}

pub fn encode_mint_program(amount: u64) -> Script {
    let argument = TransactionArgument::U64(amount);
    Script::new(vec![], vec![argument])
}

pub fn encode_transfer_program(recipient: AccountAddress, amount: u64) -> Script {
    let argument1 = TransactionArgument::Address(recipient);
    let argument2 = TransactionArgument::U64(amount);
    Script::new(vec![], vec![argument1, argument2])
}

pub fn encode_mint_transaction(sender: AccountAddress, amount: u64) -> Transaction {
    encode_transaction(sender, encode_mint_program(amount))
}

pub fn encode_transfer_transaction(
    sender: AccountAddress,
    recipient: AccountAddress,
    amount: u64,
) -> Transaction {
    encode_transaction(sender, encode_transfer_program(recipient, amount))
}

fn encode_transaction(sender: AccountAddress, program: Script) -> Transaction {
    let raw_transaction =
        RawUserTransaction::new_script(sender, 0, program, 0, 0, std::time::Duration::from_secs(0));

    let (privkey, pubkey) = compat::generate_keypair(None);
    Transaction::UserTransaction(
        raw_transaction
            .sign(&privkey, pubkey)
            .expect("Failed to sign raw transaction.")
            .into_inner(),
    )
}

fn decode_transaction(txn: &SignedUserTransaction) -> MockTransaction {
    let sender = txn.sender();
    match txn.payload() {
        TransactionPayload::Script(script) => {
            assert!(script.code().is_empty(), "Code should be empty.");
            match script.args().len() {
                1 => match script.args()[0] {
                    TransactionArgument::U64(amount) => MockTransaction::Mint { sender, amount },
                    _ => unimplemented!(
                        "Only one integer argument is allowed for mint transactions."
                    ),
                },
                2 => match (&script.args()[0], &script.args()[1]) {
                    (TransactionArgument::Address(recipient), TransactionArgument::U64(amount)) => {
                        MockTransaction::Payment {
                            sender,
                            recipient: *recipient,
                            amount: *amount,
                        }
                    }
                    _ => unimplemented!(
                        "The first argument for payment transaction must be recipient address \
                         and the second argument must be amount."
                    ),
                },
                _ => unimplemented!("Transaction must have one or two arguments.{:?}", txn),
            }
        }
        TransactionPayload::Module(_) => {
            unimplemented!("MockExecutor does not support Module transaction payload.")
        }
        TransactionPayload::StateSet(_) => {
            unimplemented!("MockExecutor does not support StateSet transaction payload.")
        }
    }
}
