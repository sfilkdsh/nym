// Copyright 2021 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: Apache-2.0

use crate::nymd::coin::Coin;
pub use crate::nymd::cosmwasm_client::client::CosmWasmClient;
use crate::nymd::error::NymdError;
use crate::nymd::NymdClient;
use async_trait::async_trait;
use contracts_common::ContractBuildInformation;
use cosmwasm_std::{Coin as CosmWasmCoin, Timestamp};
use mixnet_contract_common::MixId;
use serde::Deserialize;
use vesting_contract::vesting::Account;
use vesting_contract_common::{
    messages::QueryMsg as VestingQueryMsg, AllDelegationsResponse, DelegationTimesResponse,
    OriginalVestingResponse, Period, PledgeData, VestingDelegation,
};

#[async_trait]
pub trait VestingQueryClient {
    async fn query_vesting_contract<T>(&self, query: VestingQueryMsg) -> Result<T, NymdError>
    where
        for<'a> T: Deserialize<'a>;

    async fn get_vesting_contract_version(&self) -> Result<ContractBuildInformation, NymdError> {
        self.query_vesting_contract(VestingQueryMsg::GetContractVersion {})
            .await
    }

    async fn locked_coins(
        &self,
        address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError>;

    async fn spendable_coins(
        &self,
        vesting_account_address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError>;

    async fn vested_coins(
        &self,
        vesting_account_address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError>;

    async fn vesting_coins(
        &self,
        vesting_account_address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError>;

    async fn vesting_start_time(
        &self,
        vesting_account_address: &str,
    ) -> Result<Timestamp, NymdError>;

    async fn vesting_end_time(&self, vesting_account_address: &str)
        -> Result<Timestamp, NymdError>;

    async fn original_vesting(
        &self,
        vesting_account_address: &str,
    ) -> Result<OriginalVestingResponse, NymdError>;

    async fn delegated_free(
        &self,
        vesting_account_address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError>;

    async fn delegated_vesting(
        &self,
        vesting_account_address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError>;

    async fn get_account(&self, address: &str) -> Result<Account, NymdError>;
    async fn get_mixnode_pledge(&self, address: &str) -> Result<Option<PledgeData>, NymdError>;
    async fn get_gateway_pledge(&self, address: &str) -> Result<Option<PledgeData>, NymdError>;
    async fn get_current_vesting_period(
        &self,
        vesting_account_address: &str,
    ) -> Result<Period, NymdError>;

    async fn get_delegation_timestamps(
        &self,
        address: &str,
        mix_id: MixId,
    ) -> Result<DelegationTimesResponse, NymdError>;

    async fn get_all_vesting_delegations_paged(
        &self,
        start_after: Option<(u32, MixId, u64)>,
        limit: Option<u32>,
    ) -> Result<AllDelegationsResponse, NymdError>;

    async fn get_all_vesting_delegations(&self) -> Result<Vec<VestingDelegation>, NymdError> {
        let mut delegations = Vec::new();
        let mut start_after = None;
        loop {
            let mut paged_response = self
                .get_all_vesting_delegations_paged(start_after.take(), None)
                .await?;
            delegations.append(&mut paged_response.delegations);

            if let Some(start_after_res) = paged_response.start_next_after {
                start_after = Some(start_after_res)
            } else {
                break;
            }
        }

        Ok(delegations)
    }
}

#[async_trait]
impl<C: CosmWasmClient + Sync + Send> VestingQueryClient for NymdClient<C> {
    async fn query_vesting_contract<T>(&self, query: VestingQueryMsg) -> Result<T, NymdError>
    where
        for<'a> T: Deserialize<'a>,
    {
        self.client
            .query_contract_smart(self.vesting_contract_address(), &query)
            .await
    }

    async fn locked_coins(
        &self,
        vesting_account_address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError> {
        let request = VestingQueryMsg::LockedCoins {
            vesting_account_address: vesting_account_address.to_string(),
            block_time,
        };
        self.client
            .query_contract_smart::<_, CosmWasmCoin>(self.vesting_contract_address(), &request)
            .await
            .map(Into::into)
    }

    async fn spendable_coins(
        &self,
        vesting_account_address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError> {
        let request = VestingQueryMsg::SpendableCoins {
            vesting_account_address: vesting_account_address.to_string(),
            block_time,
        };
        self.client
            .query_contract_smart::<_, CosmWasmCoin>(self.vesting_contract_address(), &request)
            .await
            .map(Into::into)
    }
    async fn vested_coins(
        &self,
        vesting_account_address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError> {
        let request = VestingQueryMsg::GetVestedCoins {
            vesting_account_address: vesting_account_address.to_string(),
            block_time,
        };
        self.client
            .query_contract_smart::<_, CosmWasmCoin>(self.vesting_contract_address(), &request)
            .await
            .map(Into::into)
    }
    async fn vesting_coins(
        &self,
        vesting_account_address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError> {
        let request = VestingQueryMsg::GetVestingCoins {
            vesting_account_address: vesting_account_address.to_string(),
            block_time,
        };
        self.client
            .query_contract_smart::<_, CosmWasmCoin>(self.vesting_contract_address(), &request)
            .await
            .map(Into::into)
    }

    async fn vesting_start_time(
        &self,
        vesting_account_address: &str,
    ) -> Result<Timestamp, NymdError> {
        let request = VestingQueryMsg::GetStartTime {
            vesting_account_address: vesting_account_address.to_string(),
        };
        self.client
            .query_contract_smart(self.vesting_contract_address(), &request)
            .await
    }

    async fn vesting_end_time(
        &self,
        vesting_account_address: &str,
    ) -> Result<Timestamp, NymdError> {
        let request = VestingQueryMsg::GetEndTime {
            vesting_account_address: vesting_account_address.to_string(),
        };
        self.client
            .query_contract_smart(self.vesting_contract_address(), &request)
            .await
    }

    async fn original_vesting(
        &self,
        vesting_account_address: &str,
    ) -> Result<OriginalVestingResponse, NymdError> {
        let request = VestingQueryMsg::GetOriginalVesting {
            vesting_account_address: vesting_account_address.to_string(),
        };
        self.client
            .query_contract_smart(self.vesting_contract_address(), &request)
            .await
    }

    async fn delegated_free(
        &self,
        vesting_account_address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError> {
        let request = VestingQueryMsg::GetDelegatedFree {
            vesting_account_address: vesting_account_address.to_string(),
            block_time,
        };
        self.client
            .query_contract_smart::<_, CosmWasmCoin>(self.vesting_contract_address(), &request)
            .await
            .map(Into::into)
    }

    /// Returns the total amount of delegated tokens that have vested
    async fn delegated_vesting(
        &self,
        vesting_account_address: &str,
        block_time: Option<Timestamp>,
    ) -> Result<Coin, NymdError> {
        let request = VestingQueryMsg::GetDelegatedVesting {
            vesting_account_address: vesting_account_address.to_string(),
            block_time,
        };
        self.client
            .query_contract_smart::<_, CosmWasmCoin>(self.vesting_contract_address(), &request)
            .await
            .map(Into::into)
    }

    async fn get_account(&self, address: &str) -> Result<Account, NymdError> {
        let request = VestingQueryMsg::GetAccount {
            address: address.to_string(),
        };
        self.client
            .query_contract_smart(self.vesting_contract_address(), &request)
            .await
    }
    async fn get_mixnode_pledge(&self, address: &str) -> Result<Option<PledgeData>, NymdError> {
        let request = VestingQueryMsg::GetMixnode {
            address: address.to_string(),
        };
        self.client
            .query_contract_smart(self.vesting_contract_address(), &request)
            .await
    }
    async fn get_gateway_pledge(&self, address: &str) -> Result<Option<PledgeData>, NymdError> {
        let request = VestingQueryMsg::GetGateway {
            address: address.to_string(),
        };
        self.client
            .query_contract_smart(self.vesting_contract_address(), &request)
            .await
    }

    async fn get_current_vesting_period(&self, address: &str) -> Result<Period, NymdError> {
        let request = VestingQueryMsg::GetCurrentVestingPeriod {
            address: address.to_string(),
        };
        self.client
            .query_contract_smart(self.vesting_contract_address(), &request)
            .await
    }

    async fn get_delegation_timestamps(
        &self,
        address: &str,
        mix_id: MixId,
    ) -> Result<DelegationTimesResponse, NymdError> {
        let request = VestingQueryMsg::GetDelegationTimes {
            address: address.to_string(),
            mix_id,
        };
        self.client
            .query_contract_smart(self.vesting_contract_address(), &request)
            .await
    }

    async fn get_all_vesting_delegations_paged(
        &self,
        start_after: Option<(u32, MixId, u64)>,
        limit: Option<u32>,
    ) -> Result<AllDelegationsResponse, NymdError> {
        let request = VestingQueryMsg::GetAllDelegations { start_after, limit };
        self.client
            .query_contract_smart(self.vesting_contract_address(), &request)
            .await
    }
}
