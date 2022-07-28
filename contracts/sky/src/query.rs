use shade_protocol::{
    c_std::{self, Api, Extern, HumanAddr, Querier, StdError, StdResult, Storage},
    contract_interfaces::{
        dao::adapter,
        sky::{cycles::Offer, Config, Cycles, QueryAnswer, SelfAddr, ViewingKeys},
        snip20,
    },
    math_compat::Uint128,
    secret_toolkit::utils::Query,
    utils::storage::plus::ItemStorage,
};

pub fn config<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>) -> StdResult<QueryAnswer> {
    Ok(QueryAnswer::Config {
        config: Config::load(&deps.storage)?,
    })
}

pub fn get_balances<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<QueryAnswer> {
    let viewing_key = ViewingKeys::load(&deps.storage)?.0;
    let self_addr = SelfAddr::load(&deps.storage)?.0;
    let config = Config::load(&deps.storage)?;

    // Query shd balance
    let mut res = snip20::QueryMsg::Balance {
        address: self_addr.clone(),
        key: viewing_key.clone(),
    }
    .query(
        &deps.querier,
        config.shd_token.code_hash.clone(),
        config.shd_token.address.clone(),
    )?;

    let shd_bal = match res {
        snip20::QueryAnswer::Balance { amount } => amount,
        _ => Uint128::zero(),
    };

    // Query silk balance
    res = snip20::QueryMsg::Balance {
        address: self_addr.clone(),
        key: viewing_key.clone(),
    }
    .query(
        &deps.querier,
        config.silk_token.code_hash.clone(),
        config.silk_token.address.clone(),
    )?;

    let silk_bal = match res {
        snip20::QueryAnswer::Balance { amount } => amount,
        _ => Uint128::zero(),
    };

    // Query sscrt balance
    res = snip20::QueryMsg::Balance {
        address: self_addr.clone(),
        key: viewing_key.clone(),
    }
    .query(
        &deps.querier,
        config.sscrt_token.code_hash.clone(),
        config.sscrt_token.address.clone(),
    )?;

    let sscrt_bal = match res {
        snip20::QueryAnswer::Balance { amount } => amount,
        _ => Uint128::zero(),
    };

    Ok(QueryAnswer::Balance {
        shd_bal,
        silk_bal,
        sscrt_bal,
    })
}

pub fn get_cycles<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<QueryAnswer> {
    //Need to make private eventually
    Ok(QueryAnswer::GetCycles {
        cycles: Cycles::load(&deps.storage)?.0,
    })
}

pub fn swap_amount<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    index: Uint128,
    self_address: Option<HumanAddr>,
) -> StdResult<QueryAnswer> {
    let cycles = Cycles::load(&deps.storage)?.0;
    let viewing_key = ViewingKeys::load(&deps.storage)?.0;
    let config = Config::load(&deps.storage)?;
    let i = index.u128() as usize;
    if (i) >= cycles.len() {
        return Err(StdError::generic_err("Index passed is out of bounds"));
    }
    let self_addr = {
        if let Some(self_address) = self_address {
            self_address
        } else {
            SelfAddr::load(&deps.storage)?.0
        }
    };
    let res = snip20::QueryMsg::Balance {
        address: self_addr.clone(),
        key: viewing_key.clone(),
    }
    .query(
        &deps.querier,
        cycles[i].clone().start_addr.code_hash.clone(),
        cycles[i].clone().start_addr.address.clone(),
    )?;
    let max = match res {
        snip20::QueryAnswer::Balance { amount } => amount,
        _ => Uint128::zero(),
    };
    if max <= config.clone().min_amount {
        return Err(StdError::generic_err("Not enough of starting token"));
    }
    let mut pool_amounts = vec![];
    let mut max_decimals = Uint128::zero();
    for pairs in cycles[i].pair_addrs {
        if pairs.clone().token0_decimals > max_decimals.clone() {
            max_decimals = pairs.clone().token0_decimals;
        }
        if pairs.clone().token1_decimals > max_decimals.clone() {
            max_decimals = pairs.clone().token1_decimals;
        }
    }
    for pairs in cycles[i].pair_addrs {
        let mut pool_tuple = pairs.pool_amounts(deps)?;
        pool_tuple.0 =
            pool_tuple
                .0
                .checked_mul(Uint128::new(10).checked_pow(
                    max_decimals.checked_sub(pairs.token0_decimals)?.u128() as u32,
                )?)?;
        pool_tuple.1 =
            pool_tuple
                .1
                .checked_mul(Uint128::new(10).checked_pow(
                    max_decimals.checked_sub(pairs.token1_decimals)?.u128() as u32,
                )?)?;
        pool_amounts.push(pool_tuple);
    }
    let add_amount = max
        .checked_sub(config.min_amount)?
        .checked_div(Uint128::new(4))?;
    let current_swap_amount = config.min_amount.clone();
    let mut query_answer = QueryAnswer::SwapAmount {
        swap_amount: Uint128::zero(),
        is_profitable: false,
        direction: cycles[i],
        swap_amounts: vec![],
        profit: Uint128::zero(),
    };
    let last_profit = Uint128::zero();
    for i in 0..5 {
        let res = cycle_profitability(
            deps,
            current_swap_amount.clone(),
            index.clone(),
            Some(Cycles(cycles)),
        )?;
        if res.profit > last_profit {
            query_answer = QueryAnswer::SwapAmount{
                swap_amount: current_swap_amount.clone(),
                is_profitable
        }
    }

    Ok(QueryAnswer::SwapAmount {
        swap_amount: Uint128::zero(),
    })
}

pub fn cycle_profitability<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    amount: Uint128,
    index: Uint128,
    passed_cycles: Option<Cycles>,
) -> StdResult<QueryAnswer> {
    let mut cycles = {
        if let Some(passed_cycles) = passed_cycles {
            passed_cycles.0
        } else {
            Cycles::load(&deps.storage)?.0
        }
    };
    let mut swap_amounts = vec![amount];
    let i = index.u128() as usize;

    if (i) >= cycles.len() {
        return Err(StdError::generic_err("Index passed is out of bounds"));
    }

    // set up inital offer
    let mut current_offer = Offer {
        asset: cycles[i].start_addr.clone(),
        amount,
    };

    //loop through the pairs in the cycle
    for arb_pair in cycles[i].pair_addrs.clone() {
        // simulate swap will run a query with respect to which dex or minting that the pair says
        // it is
        let estimated_return =
            arb_pair
                .clone()
                .simulate_swap(&deps, current_offer.clone(), Some(true))?;
        swap_amounts.push(estimated_return.clone());
        // set up the next offer with the other token contract in the pair and the expected return
        // from the last query
        if current_offer.asset.code_hash.clone() == arb_pair.token0.code_hash.clone() {
            current_offer = Offer {
                asset: arb_pair.token1.clone(),
                amount: estimated_return,
            };
        } else {
            current_offer = Offer {
                asset: arb_pair.token0.clone(),
                amount: estimated_return,
            };
        }
    }

    /*if swap_amounts.len() > cycles[i].pair_addrs.clone().len() {
        return Err(StdError::generic_err("More swap amounts than arb pairs"));
    }*/

    // if the last calculated swap is greater than the initial amount, return true
    if current_offer.amount.u128() > amount.u128() {
        return Ok(QueryAnswer::IsCycleProfitable {
            is_profitable: true,
            direction: cycles[i].clone(),
            swap_amounts,
            profit: current_offer.amount.checked_sub(amount)?,
        });
    }
    let mut return_swap_amounts = swap_amounts;
    let last_offer_amount = current_offer.amount;

    // reset these variables in order to check the other way
    swap_amounts = vec![amount];
    current_offer = Offer {
        asset: cycles[i].start_addr.clone(),
        amount,
    };

    // this is a fancy way of iterating through a vec in reverse
    for arb_pair in cycles[i].pair_addrs.clone().iter().rev() {
        // get the estimated return from the simulate swap function
        let estimated_return =
            arb_pair
                .clone()
                .simulate_swap(&deps, current_offer.clone(), Some(true))?;
        swap_amounts.push(estimated_return.clone());
        // set the current offer to the other asset we are swapping into
        if current_offer.asset.code_hash.clone() == arb_pair.token0.code_hash.clone() {
            current_offer = Offer {
                asset: arb_pair.token1.clone(),
                amount: estimated_return,
            };
        } else {
            current_offer = Offer {
                asset: arb_pair.token0.clone(),
                amount: estimated_return,
            };
        }
    }

    // check to see if this direction was profitable
    if current_offer.amount > amount {
        // do an inplace reversal of the pair_addrs so that we know which way the opportunity goes
        cycles[i].pair_addrs.reverse();
        return Ok(QueryAnswer::IsCycleProfitable {
            is_profitable: true,
            direction: cycles[i].clone(),
            swap_amounts,
            profit: current_offer.amount.checked_sub(amount)?,
        });
    }
    if current_offer.amount > last_offer_amount {
        return_swap_amounts = swap_amounts;
    } else {
        cycles[i].pair_addrs.reverse();
    }

    // If both possible directions are unprofitable, return false
    Ok(QueryAnswer::IsCycleProfitable {
        is_profitable: false,
        direction: cycles[i].clone(),
        swap_amounts: return_swap_amounts,
        profit: Uint128::zero(),
    })
}

pub fn any_cycles_profitable<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    amount: Uint128,
) -> StdResult<QueryAnswer> {
    let cycles = Cycles::load(&deps.storage)?.0;
    let mut return_is_profitable = vec![];
    let mut return_directions = vec![];
    let mut return_swap_amounts = vec![];
    let mut return_profit = vec![];

    // loop through the cycles with an index
    for index in 0..cycles.len() {
        // for each cycle, check its profitability
        let res = cycle_profitability(deps, amount, Uint128::from(index as u128)).unwrap();
        match res {
            QueryAnswer::IsCycleProfitable {
                is_profitable,
                direction,
                swap_amounts,
                profit,
            } => {
                // push the results to a vec
                return_is_profitable.push(is_profitable);
                return_directions.push(direction);
                return_swap_amounts.push(swap_amounts);
                return_profit.push(profit);
            }
            _ => {
                return Err(StdError::generic_err("Unexpected result"));
            }
        }
    }

    Ok(QueryAnswer::IsAnyCycleProfitable {
        is_profitable: return_is_profitable,
        direction: return_directions,
        swap_amounts: return_swap_amounts,
        profit: return_profit,
    })
}

pub fn adapter_balance<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    asset: HumanAddr,
) -> StdResult<adapter::QueryAnswer> {
    let config = Config::load(&deps.storage)?;
    let viewing_key = ViewingKeys::load(&deps.storage)?.0;
    let self_addr = SelfAddr::load(&deps.storage)?.0;

    let contract;
    if config.shd_token.address == asset {
        contract = config.shd_token.clone();
    } else if config.silk_token.address == asset {
        contract = config.silk_token.clone();
    } else if config.sscrt_token.address == asset {
        contract = config.sscrt_token.clone();
    } else {
        return Ok(adapter::QueryAnswer::Unbondable {
            amount: c_std::Uint128::zero(),
        });
    }

    let res = snip20::QueryMsg::Balance {
        address: self_addr.clone(),
        key: viewing_key.clone(),
    }
    .query(
        &deps.querier,
        contract.code_hash.clone(),
        contract.address.clone(),
    )?;

    let amount = match res {
        snip20::QueryAnswer::Balance { amount } => amount,
        _ => Uint128::zero(),
    };

    Ok(adapter::QueryAnswer::Unbondable {
        amount: c_std::Uint128(amount.u128()),
    })
}

pub fn adapter_claimable<S: Storage, A: Api, Q: Querier>(
    _deps: &Extern<S, A, Q>,
    _asset: HumanAddr,
) -> StdResult<adapter::QueryAnswer> {
    Ok(adapter::QueryAnswer::Claimable {
        amount: c_std::Uint128::zero(),
    })
}

// Same as adapter_balance
pub fn adapter_unbondable<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    asset: HumanAddr,
) -> StdResult<adapter::QueryAnswer> {
    let config = Config::load(&deps.storage)?;
    let viewing_key = ViewingKeys::load(&deps.storage)?.0;
    let self_addr = SelfAddr::load(&deps.storage)?.0;

    let contract;
    if config.shd_token.address == asset {
        contract = config.shd_token.clone();
    } else if config.silk_token.address == asset {
        contract = config.silk_token.clone();
    } else if config.sscrt_token.address == asset {
        contract = config.sscrt_token.clone();
    } else {
        return Ok(adapter::QueryAnswer::Unbondable {
            amount: c_std::Uint128::zero(),
        });
    }

    let res = snip20::QueryMsg::Balance {
        address: self_addr.clone(),
        key: viewing_key.clone(),
    }
    .query(
        &deps.querier,
        contract.code_hash.clone(),
        contract.address.clone(),
    )?;

    let amount = match res {
        snip20::QueryAnswer::Balance { amount } => amount,
        _ => Uint128::zero(),
    };

    Ok(adapter::QueryAnswer::Unbondable {
        amount: c_std::Uint128(amount.u128()),
    })
}

pub fn adapter_unbonding<S: Storage, A: Api, Q: Querier>(
    _deps: &Extern<S, A, Q>,
    _asset: HumanAddr,
) -> StdResult<adapter::QueryAnswer> {
    Ok(adapter::QueryAnswer::Unbonding {
        amount: c_std::Uint128::zero(),
    })
}

// Same as adapter_balance
pub fn adapter_reserves<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    asset: HumanAddr,
) -> StdResult<adapter::QueryAnswer> {
    let config = Config::load(&deps.storage)?;
    let viewing_key = ViewingKeys::load(&deps.storage)?.0;
    let self_addr = SelfAddr::load(&deps.storage)?.0;

    let contract;
    if config.shd_token.address == asset {
        contract = config.shd_token.clone();
    } else if config.silk_token.address == asset {
        contract = config.silk_token.clone();
    } else if config.sscrt_token.address == asset {
        contract = config.sscrt_token.clone();
    } else {
        return Ok(adapter::QueryAnswer::Unbondable {
            amount: c_std::Uint128::zero(),
        });
    }

    let res = snip20::QueryMsg::Balance {
        address: self_addr.clone(),
        key: viewing_key.clone(),
    }
    .query(
        &deps.querier,
        contract.code_hash.clone(),
        contract.address.clone(),
    )?;

    let amount = match res {
        snip20::QueryAnswer::Balance { amount } => amount,
        _ => Uint128::zero(),
    };

    Ok(adapter::QueryAnswer::Unbondable {
        amount: c_std::Uint128(amount.u128()),
    })
}
