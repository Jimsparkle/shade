use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{
    Api, CanonicalAddr, Coin, HumanAddr, ReadonlyStorage, StdError, StdResult, Storage,
};
use secret_storage_plus::{Item, Map};
use cosmwasm_math_compat::Uint128;

use crate::utils::storage::plus::{ItemStorage, MapStorage, NaiveMapStorage};

// Note that id is a globally incrementing counter.
// Since it's 64 bits long, even at 50 tx/s it would take
// over 11 billion years for it to rollback. I'm pretty sure
// we'll have bigger issues by then.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Tx {
    pub id: u64,
    pub from: HumanAddr,
    pub sender: HumanAddr,
    pub receiver: HumanAddr,
    pub coins: Coin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    // The block time and block height are optional so that the JSON schema
    // reflects that some SNIP-20 contracts may not include this info.
    pub block_time: Option<u64>,
    pub block_height: Option<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TxAction {
    Transfer {
        from: HumanAddr,
        sender: HumanAddr,
        recipient: HumanAddr,
    },
    Mint {
        minter: HumanAddr,
        recipient: HumanAddr,
    },
    Burn {
        burner: HumanAddr,
        owner: HumanAddr,
    },
    Deposit {},
    Redeem {},
}

// Note that id is a globally incrementing counter.
// Since it's 64 bits long, even at 50 tx/s it would take
// over 11 billion years for it to rollback. I'm pretty sure
// we'll have bigger issues by then.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct RichTx {
    pub id: u64,
    pub action: TxAction,
    pub coins: Coin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    pub block_time: u64,
    pub block_height: u64,
}

// Stored types:

/// This type is the stored version of the legacy transfers
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
struct StoredLegacyTransfer {
    id: u64,
    from: HumanAddr,
    sender: HumanAddr,
    receiver: HumanAddr,
    coins: Coin,
    memo: Option<String>,
    block_time: u64,
    block_height: u64,
}

impl StoredLegacyTransfer {
    pub fn into_humanized<>(self) -> StdResult<Tx> {
        let tx = Tx {
            id: self.id,
            from: self.from,
            sender: self.sender,
            receiver: self.receiver,
            coins: self.coins,
            memo: self.memo,
            block_time: Some(self.block_time),
            block_height: Some(self.block_height),
        };
        Ok(tx)
    }

    fn append<S: Storage>(
        &self,
        storage: &mut S,
        for_address: &HumanAddr,
    ) -> StdResult<()> {
        let mut id = UserTXTotal::may_load(
            storage,
            USER_TRANSFER_INDEX,
            for_address.clone()
        )?.unwrap_or(UserTXTotal(0)).0;

        UserTXTotal(id + 1).save(storage, USER_TRANSFER_INDEX, for_address.clone())?;
        self.save(storage, (for_address.clone(), id))
    }
}

impl MapStorage<'static, (HumanAddr, u64)> for StoredLegacyTransfer {
    const MAP: Map<'static, (HumanAddr, u64), Self> = Map::new("stored-legacy-transfer-");
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
enum TxCode {
    Transfer = 0,
    Mint = 1,
    Burn = 2,
    Deposit = 3,
    Redeem = 4,
}

impl TxCode {
    fn to_u8(self) -> u8 {
        self as u8
    }

    fn from_u8(n: u8) -> StdResult<Self> {
        use TxCode::*;
        match n {
            0 => Ok(Transfer),
            1 => Ok(Mint),
            2 => Ok(Burn),
            3 => Ok(Deposit),
            4 => Ok(Redeem),
            other => Err(StdError::generic_err(format!(
                "Unexpected Tx code in transaction history: {} Storage is corrupted.",
                other
            ))),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
struct StoredTxAction {
    tx_type: u8,
    address1: Option<HumanAddr>,
    address2: Option<HumanAddr>,
    address3: Option<HumanAddr>,
}

impl StoredTxAction {
    fn transfer(from: HumanAddr, sender: HumanAddr, recipient: HumanAddr) -> Self {
        Self {
            tx_type: TxCode::Transfer.to_u8(),
            address1: Some(from),
            address2: Some(sender),
            address3: Some(recipient),
        }
    }
    fn mint(minter: HumanAddr, recipient: HumanAddr) -> Self {
        Self {
            tx_type: TxCode::Mint.to_u8(),
            address1: Some(minter),
            address2: Some(recipient),
            address3: None,
        }
    }
    fn burn(owner: HumanAddr, burner: HumanAddr) -> Self {
        Self {
            tx_type: TxCode::Burn.to_u8(),
            address1: Some(burner),
            address2: Some(owner),
            address3: None,
        }
    }
    fn deposit() -> Self {
        Self {
            tx_type: TxCode::Deposit.to_u8(),
            address1: None,
            address2: None,
            address3: None,
        }
    }
    fn redeem() -> Self {
        Self {
            tx_type: TxCode::Redeem.to_u8(),
            address1: None,
            address2: None,
            address3: None,
        }
    }

    fn into_humanized<>(self) -> StdResult<TxAction> {
        let transfer_addr_err = || {
            StdError::generic_err(
                "Missing address in stored Transfer transaction. Storage is corrupt",
            )
        };
        let mint_addr_err = || {
            StdError::generic_err("Missing address in stored Mint transaction. Storage is corrupt")
        };
        let burn_addr_err = || {
            StdError::generic_err("Missing address in stored Burn transaction. Storage is corrupt")
        };

        // In all of these, we ignore fields that we don't expect to find populated
        let action = match TxCode::from_u8(self.tx_type)? {
            TxCode::Transfer => {
                let from = self.address1.ok_or_else(transfer_addr_err)?;
                let sender = self.address2.ok_or_else(transfer_addr_err)?;
                let recipient = self.address3.ok_or_else(transfer_addr_err)?;
                TxAction::Transfer {
                    from,
                    sender,
                    recipient,
                }
            }
            TxCode::Mint => {
                let minter = self.address1.ok_or_else(mint_addr_err)?;
                let recipient = self.address2.ok_or_else(mint_addr_err)?;
                TxAction::Mint { minter, recipient }
            }
            TxCode::Burn => {
                let burner = self.address1.ok_or_else(burn_addr_err)?;
                let owner = self.address2.ok_or_else(burn_addr_err)?;
                TxAction::Burn { burner, owner }
            }
            TxCode::Deposit => TxAction::Deposit {},
            TxCode::Redeem => TxAction::Redeem {},
        };

        Ok(action)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
struct StoredRichTx {
    id: u64,
    action: StoredTxAction,
    coins: Coin,
    memo: Option<String>,
    block_time: u64,
    block_height: u64,
}

impl StoredRichTx {
    fn new(
        id: u64,
        action: StoredTxAction,
        coins: Coin,
        memo: Option<String>,
        block: &cosmwasm_std::BlockInfo,
    ) -> Self {
        Self {
            id,
            action,
            coins,
            memo,
            block_time: block.time,
            block_height: block.height,
        }
    }

    fn into_humanized<>(self) -> StdResult<RichTx> {
        Ok(RichTx {
            id: self.id,
            action: self.action.into_humanized()?,
            coins: self.coins,
            memo: self.memo,
            block_time: self.block_time,
            block_height: self.block_height,
        })
    }

    fn from_stored_legacy_transfer(transfer: StoredLegacyTransfer) -> Self {
        let action = StoredTxAction::transfer(transfer.from, transfer.sender, transfer.receiver);
        Self {
            id: transfer.id,
            action,
            coins: transfer.coins,
            memo: transfer.memo,
            block_time: transfer.block_time,
            block_height: transfer.block_height,
        }
    }

    fn append<S: Storage>(
        &self,
        storage: &mut S,
        for_address: &HumanAddr,
    ) -> StdResult<()> {
        let mut id = UserTXTotal::may_load(
            storage,
            USER_TX_INDEX,
            for_address.clone()
        )?.unwrap_or(UserTXTotal(0)).0;

        UserTXTotal(id + 1).save(storage, USER_TX_INDEX, for_address.clone())?;
        self.save(storage, (for_address.clone(), id))
    }
}

impl MapStorage<'static, (HumanAddr, u64)> for StoredRichTx {
    const MAP: Map<'static, (HumanAddr, u64), Self> = Map::new("stored-rich-tx-");
}

// Storage functions:
#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
struct TXCount(pub u64);

impl ItemStorage for TXCount {
    const ITEM: Item<'static, Self> = Item::new("tx-count-");
}

fn increment_tx_count<S: Storage>(storage: &mut S) -> StdResult<u64> {
    let id = TXCount::load(storage)?.0 + 1;
    TXCount(id).save(storage)?;
    Ok(id)
}

// User tx index
#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
struct UserTXTotal(pub u64);

impl NaiveMapStorage<'static> for UserTXTotal {}
const USER_TX_INDEX: Map<'static, HumanAddr, UserTXTotal> = Map::new("user-tx-index-");
const USER_TRANSFER_INDEX: Map<'static, HumanAddr, UserTXTotal> = Map::new("user-transfer-index-");

#[allow(clippy::too_many_arguments)] // We just need them
pub fn store_transfer<S: Storage>(
    storage: &mut S,
    owner: &HumanAddr,
    sender: &HumanAddr,
    receiver: &HumanAddr,
    amount: Uint128,
    denom: String,
    memo: Option<String>,
    block: &cosmwasm_std::BlockInfo,
) -> StdResult<()> {
    let id = increment_tx_count(storage)?;
    let coins = Coin { denom, amount: amount.into() };
    let transfer = StoredLegacyTransfer {
        id,
        from: owner.clone(),
        sender: sender.clone(),
        receiver: receiver.clone(),
        coins,
        memo,
        block_time: block.time,
        block_height: block.height,
    };
    let tx = StoredRichTx::from_stored_legacy_transfer(transfer.clone());

    // Write to the owners history if it's different from the other two addresses
    if owner != sender && owner != receiver {
        // cosmwasm_std::debug_print("saving transaction history for owner");
        tx.append(storage, owner)?;
        transfer.append(storage, owner)?;
    }
    // Write to the sender's history if it's different from the receiver
    if sender != receiver {
        // cosmwasm_std::debug_print("saving transaction history for sender");
        tx.append(storage, sender)?;
        transfer.append(storage, sender)?;
    }
    // Always write to the recipient's history
    // cosmwasm_std::debug_print("saving transaction history for receiver");
    tx.append(storage, receiver)?;
    transfer.append(storage, receiver)?;

    Ok(())
}

pub fn store_mint<S: Storage>(
    storage: &mut S,
    minter: &HumanAddr,
    recipient: &HumanAddr,
    amount: Uint128,
    denom: String,
    memo: Option<String>,
    block: &cosmwasm_std::BlockInfo,
) -> StdResult<()> {
    let id = increment_tx_count(storage)?;
    let coins = Coin { denom, amount: amount.into() };
    let action = StoredTxAction::mint(minter.clone(), recipient.clone());
    let tx = StoredRichTx::new(id, action, coins, memo, block);

    if minter != recipient {
        tx.append(storage, recipient)?;
    }
    tx.append(storage, minter)?;

    Ok(())
}

pub fn store_burn<S: Storage>(
    storage: &mut S,
    owner: &HumanAddr,
    burner: &HumanAddr,
    amount: Uint128,
    denom: String,
    memo: Option<String>,
    block: &cosmwasm_std::BlockInfo,
) -> StdResult<()> {
    let id = increment_tx_count(storage)?;
    let coins = Coin { denom, amount: amount.into() };
    let action = StoredTxAction::burn(owner.clone(), burner.clone());
    let tx = StoredRichTx::new(id, action, coins, memo, block);

    if burner != owner {
        tx.append(storage, owner)?;
    }
    tx.append(storage, burner)?;

    Ok(())
}

pub fn store_deposit<S: Storage>(
    storage: &mut S,
    recipient: &HumanAddr,
    amount: Uint128,
    denom: String,
    block: &cosmwasm_std::BlockInfo,
) -> StdResult<()> {
    let id = increment_tx_count(storage)?;
    let coins = Coin { denom, amount: amount.into() };
    let action = StoredTxAction::deposit();
    let tx = StoredRichTx::new(id, action, coins, None, block);

    tx.append(storage, recipient)?;

    Ok(())
}

pub fn store_redeem<S: Storage>(
    storage: &mut S,
    redeemer: &HumanAddr,
    amount: Uint128,
    denom: String,
    block: &cosmwasm_std::BlockInfo,
) -> StdResult<()> {
    let id = increment_tx_count(storage)?;
    let coins = Coin { denom, amount: amount.into() };
    let action = StoredTxAction::redeem();
    let tx = StoredRichTx::new(id, action, coins, None, block);

    tx.append(storage, redeemer)?;

    Ok(())
}

pub fn get_txs<S: Storage>(
    storage: &S,
    for_address: &HumanAddr,
    page: u32,
    page_size: u32,
) -> StdResult<(Vec<RichTx>, u64)> {
    let id = UserTXTotal::load(storage, USER_TX_INDEX, for_address.clone())?.0;
    let start_index = page as u64 * page_size as u64;
    let size: u64;
    if (start_index + page_size as u64) > id {
        size = id;
    }
    else {
        size = page_size as u64 + start_index;
    }

    let mut txs = vec![];
    for index in start_index..size {
        let stored_tx = StoredRichTx::load(storage, (for_address.clone(), index))?;
        txs.push(stored_tx.into_humanized()?);
    }

    Ok((txs, size-start_index))
}

pub fn get_transfers<S: Storage>(
    storage: &S,
    for_address: &HumanAddr,
    page: u32,
    page_size: u32,
) -> StdResult<(Vec<Tx>, u64)> {
    let id = UserTXTotal::load(storage, USER_TRANSFER_INDEX, for_address.clone())?.0;
    let start_index = page as u64 * page_size as u64;
    let size: u64;
    if (start_index + page_size as u64) > id {
        size = id;
    }
    else {
        size = page_size as u64 + start_index;
    }

    let mut txs = vec![];
    for index in start_index..size {
        let stored_tx = StoredLegacyTransfer::load(storage, (for_address.clone(), index))?;
        txs.push(stored_tx.into_humanized()?);
    }

    Ok((txs, size-start_index))
}