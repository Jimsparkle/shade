use std::{
    ops::*,
    convert::{TryFrom, TryInto},
};
use shade_protocol::{
	c_std::{
        Addr, 
        Decimal,
        Deps,
        Isqrt,
        StdError, 
        StdResult,
        Uint128,
        Uint256,
    },
	contract_interfaces::{
        dao::adapter,
        sky::{
            cycles::{
                ArbPair,
                Derivative,
            },
            sky_derivatives::{
                Config,
		        Direction,
                DexPairs,
		        QueryAnswer,
                Rollover,
            },
        },
	},
    utils::storage::plus::ItemStorage,
};
use cosmwasm_floating_point::float::Float;

pub fn config(deps: Deps) -> StdResult<QueryAnswer> {
    Ok(QueryAnswer::Config {
        config: Config::load(deps.storage)?,
    })
}

pub fn dex_pairs(deps: Deps) -> StdResult<QueryAnswer> {
    Ok(QueryAnswer::DexPairs {
        dex_pairs: DexPairs::load(deps.storage)?.0,
    })
}

pub fn current_rollover(deps: Deps) -> StdResult<QueryAnswer> {
    Ok(QueryAnswer::CurrentRollover {
        rollover: Rollover::load(deps.storage)?.0,
    })
}
 
pub fn is_profitable(
    deps: Deps,
    pair_index: usize,
    max_swap: Option<Uint128>,
) -> StdResult<QueryAnswer> {
    let dex_pairs = DexPairs::load(deps.storage)?.0;
    if pair_index >= dex_pairs.len() {
        return Err(StdError::generic_err(format!("Invalid dex_pair index: {}", pair_index)));
    }

    let config = Config::load(deps.storage)?;
    let arb_pair = dex_pairs[pair_index].clone();
    let dex_pools = query_dex_pool(deps, arb_pair)?;
    let derivative_price: Float = query_derivative_price(config.derivative, deps)?;
    let max_swap = max_swap.and_then(|max| Some(Float::from(max)));

    // Subtracts will not overflow if trading fees are properly checked
    let unbond_rate: Float = Float::from(Decimal::one() - config.trading_fees.unbond_fee);
    let stake_rate: Float = Float::from(Decimal::one() - config.trading_fees.stake_fee);
    let dex_rate: Float = Float::from(Decimal::one() - config.trading_fees.dex_fee);

    optimization_math(dex_pools, derivative_price, unbond_rate, stake_rate, dex_rate, max_swap)
}

// Calculate optimal amounts for arbitrage, equations obtained by finding the zero of the
// derivative of the constant product equation for the two exchange operations:
// 
//     unbond_optimal_amount = sqrt(dex_pools.0 * dex_pools.1 * derivative_price * dex_rate *
//                                  unbond_rate) - dex_pools.0
//     stake_optimal_amount  = (derivative_price / stake_rate) * (sqrt(dex_pools.0 * dex_pools.1 *
//                                  dex_rate * stake_rate / stake_price) - dex_pools.1)
// 
// Where unbond means: buy on dex, then start derivative unbond
//    and stake means: mint derivative, then sell on dex
// If either of these values are positive (they should never both be positive) there is a
// profitable trade in that direction
pub fn optimization_math(
    dex_pools: (Float, Float),
    derivative_price: Float,
    unbond_rate: Float,
    stake_rate: Float,
    dex_rate: Float,
    max_swap: Option<Float>,
) -> StdResult<QueryAnswer> {
    // Float used here for easy math
    // Checked math not used because of the absurd range of Float
    let common_radical = dex_pools.0 * dex_pools.1 * dex_rate;
	let unbond_optimal_amount = (common_radical * derivative_price * unbond_rate)
                                    .sqrt()
                                    .checked_sub(dex_pools.0);
	match unbond_optimal_amount {
		Ok(amount) => {
            let swap_amount = match max_swap {
                Some(max) => Float::max(amount, max),
                None => amount,
            };
            // derivative resulting from dex swap
            let expected_return_1 = cp_result(
                                        swap_amount,
                                        dex_pools.0, 
                                        dex_pools.1,
                                        dex_rate,
                                    )?;
            // base currency resulting from unbond
            let expected_return_2 = expected_return_1 * derivative_price * unbond_rate;
			return Ok(QueryAnswer::IsProfitable {
				is_profitable: true,
                swap_amounts: Some((
                                   swap_amount.try_into()?, 
                                   expected_return_1.try_into()?, 
                                   expected_return_2.try_into()?,
                )),
				direction: Some(Direction::Unbond),
			})
		},
		Err(_err) => { }, // unbond optimal amount negative, not profitable here
	};

	let stake_optimal_inner = (common_radical * stake_rate / derivative_price)
                                    .sqrt()
                                    .checked_sub(dex_pools.1);
	match stake_optimal_inner {
		Ok(amount) => {
			let optimal_amount = derivative_price / stake_rate * amount;
            let swap_amount = match max_swap {
               Some(max) => Float::max(optimal_amount, max),
               None => optimal_amount,
            };
            
            // derivative resulting from derivative mint/stake
            let expected_return_1 = swap_amount / derivative_price * stake_rate;
            // base currency resulting from dex swap
            let expected_return_2 = cp_result(
                                        expected_return_1, 
                                        dex_pools.1, 
                                        dex_pools.0, 
                                        dex_rate
                                    )?;
			Ok(QueryAnswer::IsProfitable {
				is_profitable: true,
                swap_amounts: Some((
                                   swap_amount.try_into()?, 
                                   expected_return_1.try_into()?,
                                   expected_return_2.try_into()?,
                )),
				direction: Some(Direction::Stake),
			})
		},
		Err(_err) => Ok(QueryAnswer::IsProfitable { // mint optimal amount negative,
			is_profitable: false,                   // no profitable options
            swap_amounts: None,
			direction: None,
		})
	}
}

pub fn is_any_pair_profitable(
    deps: Deps,
    max_swap: Option<Uint128>,
) -> StdResult<QueryAnswer> {
    let pairs = DexPairs::load(deps.storage)?.0;
    if pairs.len() == 0 {
        return Err(StdError::generic_err("No dex pairs to arb!"));
    }

    let mut is_profitable_vec = vec![];
    let mut swap_amounts_vec = vec![];
    let mut direction_vec = vec![];
    for index in 0..pairs.len() {
        match is_profitable(deps, index, max_swap)? {
            QueryAnswer::IsProfitable { is_profitable, swap_amounts, direction} => {
                is_profitable_vec.push(is_profitable);
                swap_amounts_vec.push(swap_amounts);
                direction_vec.push(direction);
            },
            _ => {
                return Err(StdError::generic_err("Unexpected query answer")); // This shouln't happen
            }
        };
    }
    
    Ok(QueryAnswer::IsAnyPairProfitable {
        is_profitable: is_profitable_vec,
        swap_amounts: swap_amounts_vec,
        direction: direction_vec,
    })
}

pub fn adapter_balance(deps: Deps, asset: Addr) -> StdResult<adapter::QueryAnswer> {

    // TODO

    Ok(adapter::QueryAnswer::Balance {
        amount: shade_protocol::c_std::Uint128::zero(),
    })
}

pub fn adapter_claimable(deps: Deps, asset: Addr) -> StdResult<adapter::QueryAnswer> {

    // TODO

    Ok(adapter::QueryAnswer::Claimable {
        amount: shade_protocol::c_std::Uint128::zero(),
    })
}

pub fn adapter_unbonding(deps: Deps, asset: Addr) -> StdResult<adapter::QueryAnswer> {

    // TODO

    Ok(adapter::QueryAnswer::Unbonding {
        amount: shade_protocol::c_std::Uint128::zero(),
    })
}

pub fn adapter_unbondable(deps: Deps, asset: Addr) -> StdResult<adapter::QueryAnswer> {

    // TODO

    Ok(adapter::QueryAnswer::Unbondable {
        amount: shade_protocol::c_std::Uint128::zero(),
    })
}

pub fn adapter_reserves(deps: Deps, asset: Addr) -> StdResult<adapter::QueryAnswer> {

    // TODO

    Ok(adapter::QueryAnswer::Reserves {
        amount: shade_protocol::c_std::Uint128::zero(),
    })
}


/// Constant Product Rule similator
fn cp_result(
    amount: Float, 
    pool_1: Float, 
    pool_2: Float, 
    swap_fee: Float
) -> StdResult<Float> {
    let expected_res = pool_2 - (pool_1 * pool_2) / (pool_1 + amount);
    Ok(expected_res * swap_fee)
}

// Queries pool amounts for dex pair and divides by the token decimals to convert to float
fn query_dex_pool(deps: Deps, mut dex_pair: ArbPair) -> StdResult<(Float, Float)> {
    let config = Config::load(deps.storage)?;
    let dex_pool_amts = dex_pair.pool_amounts(deps)?;
    if dex_pair.token0 == config.derivative.contract {
        Ok((
            Float::from(dex_pool_amts.0),
            Float::from(dex_pool_amts.1),
        ))
    } 
    else if dex_pair.token0 == config.derivative.original_token {
        Ok((
            Float::from(dex_pool_amts.1),
            Float::from(dex_pool_amts.0),
        ))
    } 
    else {
        return Err(StdError::generic_err("Invalid dex_pair config"));
    }
}

fn query_derivative_price(derivative: Derivative, deps: Deps) -> StdResult<Float> {
    Ok(Float::from(derivative.query_exchange_price(deps)?))
}
