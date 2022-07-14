use shade_protocol::c_std::{Addr, Storage, Uint128};
use shade_protocol::storage::{bucket, bucket_read, Bucket, ReadonlyBucket, singleton, singleton_read, ReadonlySingleton, Singleton};
use shade_protocol::contract_interfaces::dao::lp_shade_swap;

pub static CONFIG_KEY: &[u8] = b"config";
pub static SELF_ADDRESS: &[u8] = b"self_address";
pub static VIEWING_KEY: &[u8] = b"viewing_key";
pub static UNBONDING: &[u8] = b"unbonding";

pub fn config_w<S: Storage>(storage: &mut S) -> Singleton<S, lp_shade_swap::Config> {
    singleton(storage, CONFIG_KEY)
}

pub fn config_r<S: Storage>(storage: &dyn Storage) -> ReadonlySingleton<S, lp_shade_swap::Config> {
    singleton_read(storage, CONFIG_KEY)
}

pub fn self_address_w<S: Storage>(storage: &mut S) -> Singleton<S, Addr> {
    singleton(storage, SELF_ADDRESS)
}

pub fn self_address_r<S: Storage>(storage: &dyn Storage) -> ReadonlySingleton<S, Addr> {
    singleton_read(storage, SELF_ADDRESS)
}

pub fn viewing_key_w<S: Storage>(storage: &mut S) -> Singleton<S, String> {
    singleton(storage, VIEWING_KEY)
}

pub fn viewing_key_r<S: Storage>(storage: &dyn Storage) -> ReadonlySingleton<S, String> {
    singleton_read(storage, VIEWING_KEY)
}

pub fn unbonding_w<S: Storage>(storage: &mut S) -> Bucket<S, Uint128> {
    bucket(UNBONDING, storage)
}

pub fn unbonding_r<S: Storage>(storage: &dyn Storage) -> ReadonlyBucket<S, Uint128> {
    bucket_read(UNBONDING, storage)
}
