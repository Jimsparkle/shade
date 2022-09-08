use shade_protocol::{
    c_std::{
        Api, Addr, Querier, StdError, 
        StdResult, Storage, Uint128, Deps,
    },
    snip20::helpers::{allowance_query, balance_query},
    dao::{
        adapter,
        manager,
        treasury_manager,
    },
};

use crate::storage::*;

pub fn config(
    deps: Deps,
) -> StdResult<treasury_manager::QueryAnswer> {
    Ok(treasury_manager::QueryAnswer::Config {
        config: CONFIG.load(deps.storage)?,
    })
}

pub fn pending_allowance(
    deps: Deps,
    asset: Addr,
) -> StdResult<treasury_manager::QueryAnswer> {
    let config = CONFIG.load(deps.storage)?;
    let full_asset = match ASSETS.may_load(deps.storage, asset)? {
        Some(a) => a,
        None => {
            return Err(StdError::generic_err(""));
        }
    };

    let allowance = allowance_query(
        &deps.querier,
        config.treasury,
        SELF_ADDRESS.load(deps.storage)?,
        VIEWING_KEY.load(deps.storage)?,
        1,
        &full_asset.contract.clone(),
    )?.amount;

    Ok(treasury_manager::QueryAnswer::PendingAllowance {
        amount: allowance,
    })
}

pub fn reserves(
    deps: Deps,
    asset: Addr,
    holder: Addr,
) -> StdResult<manager::QueryAnswer> {
    if let Some(full_asset) = ASSETS.may_load(deps.storage, asset)? {
        let reserves = balance_query(
            &deps.querier,
            SELF_ADDRESS.load(deps.storage)?,
            VIEWING_KEY.load(deps.storage)?,
            &full_asset.contract.clone(),
        )?;

        return Ok(manager::QueryAnswer::Reserves { 
            amount: reserves,
        });
    }

    Err(StdError::generic_err("Not a registered asset"))
}

pub fn assets(
    deps: Deps,
) -> StdResult<treasury_manager::QueryAnswer> {
    Ok(treasury_manager::QueryAnswer::Assets {
        assets: ASSET_LIST.load(deps.storage)?,
    })
}

pub fn allocations(
    deps: Deps,
    asset: Addr,
) -> StdResult<treasury_manager::QueryAnswer> {
    Ok(treasury_manager::QueryAnswer::Allocations {
        allocations: match ALLOCATIONS.may_load(deps.storage, asset)? {
            None => vec![],
            Some(a) => a,
        },
    })
}

pub fn unbonding(
    deps: Deps,
    asset: Addr,
    holder: Addr,
) -> StdResult<manager::QueryAnswer> {

    if ASSETS.may_load(deps.storage, asset.clone())?.is_none() {
        return Err(StdError::generic_err("Not an asset"));
    }

    //let allocations = allocations_r(deps.storage).load(asset.to_string().as_bytes())?;

    let _config = CONFIG.load(deps.storage)?;

    match HOLDING.may_load(deps.storage, holder)? {
        Some(holder) => {
            Ok(manager::QueryAnswer::Unbonding {
                amount: match holder.unbondings.iter().find(|u| u.token == asset.clone()) {
                    Some(u) => u.amount,
                    None => Uint128::zero(),
                }
            })
        },
        None => {
            return Err(StdError::generic_err("Invalid holder"));
        }
    }
}

pub fn claimable(
    deps: Deps,
    asset: Addr,
    holder: Addr,
) -> StdResult<manager::QueryAnswer> {

    let full_asset = match ASSETS.may_load(deps.storage, asset.clone())? {
        Some(a) => a,
        None => {
            return Err(StdError::generic_err("Not an asset"));
        }
    };
    let allocations = match ALLOCATIONS.may_load(deps.storage, asset.clone())? {
        Some(a) => a,
        None => { return Err(StdError::generic_err("Not an asset")); }
    };
    //TODO claiming needs ordered unbondings so other holders don't get bumped

    let mut claimable = balance_query(
        &deps.querier,
        SELF_ADDRESS.load(deps.storage)?,
        VIEWING_KEY.load(deps.storage)?,
        &full_asset.contract.clone(),
    )?;

    /*
    let _config = config_r(deps.storage).load()?;
    let _other_unbondings = Uint128::zero();
    */

    for alloc in allocations {
        claimable += adapter::claimable_query(deps.querier,
                              &asset, alloc.contract.clone())?;
    }

    //TODO other unbondings
    match HOLDING.may_load(deps.storage, holder)? {
        Some(holder) => {
            let unbonding = match holder.unbondings.iter().find(|u| u.token == asset) {
                Some(u) => u.amount,
                None => Uint128::zero(),
            };

            if claimable > unbonding {
                Ok(manager::QueryAnswer::Claimable {
                    amount: unbonding,
                })
            }
            else {
                Ok(manager::QueryAnswer::Claimable {
                    amount: claimable,
                })
            }
        }
        None => Err(StdError::generic_err("Invalid holder")),
    }
}

/*NOTE Could be a situation where can_unbond returns true
 * but only partial balance available for unbond resulting
 * in stalled treasury trying to unbond more than is available
 */
pub fn unbondable(
    deps: Deps,
    asset: Addr,
    holder: Addr,
) -> StdResult<manager::QueryAnswer> {

    if let Some(full_asset) = ASSETS.may_load(deps.storage, asset.clone())? {
        let config = CONFIG.load(deps.storage)?;
        let allocations = match ALLOCATIONS.may_load(deps.storage, asset.clone())? {
            Some(a) => a,
            None => { return Err(StdError::generic_err("Not an asset")); }
        };

        /*
        let unbonder = match holder {
            Some(h) => h,
            None => config.treasury,
        };
        */

        let mut balance = Uint128::zero();
        let mut unbonding = Uint128::zero();

        match HOLDING.may_load(deps.storage, holder)? {
            Some(h) => {
                if let Some(u) = h.unbondings.iter().find(|u| u.token == asset.clone()) {
                    unbonding += u.amount;
                }
                if let Some(b) = h.balances.iter().find(|b| b.token == asset.clone()) {
                    balance += b.amount;
                }
            }
            None => {
                return Err(StdError::generic_err("Invalid holder"));
            }
        }

        let mut unbondable = balance_query(
            &deps.querier,
            SELF_ADDRESS.load(deps.storage)?,
            VIEWING_KEY.load(deps.storage)?,
            &full_asset.contract.clone(),
        )?;

        for alloc in allocations {
            /*
            if unbondable >= (balance - unbonding)? {
                unbondable = (balance - unbonding)?;
                break;
            }
            */
            unbondable += adapter::unbondable_query(deps.querier,
                                  &asset, alloc.contract.clone())?;
        }

        return Ok(manager::QueryAnswer::Unbondable {
            amount: unbondable,
        });
    }

    Err(StdError::generic_err("Not a registered asset"))
}

pub fn balance(
    deps: Deps,
    asset: Addr,
    holder: Addr,
) -> StdResult<manager::QueryAnswer> {

    match ASSETS.may_load(deps.storage, asset)? {
        Some(asset) => {

            let holding = HOLDING.load(deps.storage, holder)?;

            Ok(manager::QueryAnswer::Balance {
                amount: match holding.balances.iter().find(|u| u.token == asset.contract.address) {
                    Some(b) => b.amount,
                    None => Uint128::zero(),
                }
            })

        },
        None => Err(StdError::generic_err("Not a registered asset"))
    }
}

pub fn holders(
    deps: Deps,
) -> StdResult<treasury_manager::QueryAnswer> {
    Ok(treasury_manager::QueryAnswer::Holders {
        holders: HOLDERS.load(deps.storage)?,
    })
}

pub fn holding(
    deps: Deps,
    holder: Addr,
) -> StdResult<treasury_manager::QueryAnswer> {
    match HOLDING.may_load(deps.storage, holder)? {
        Some(h) => Ok(treasury_manager::QueryAnswer::Holding { holding: h }),
        None => Err(StdError::generic_err("Not a holder")),
    }
}