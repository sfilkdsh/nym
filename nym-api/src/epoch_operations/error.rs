// Copyright 2021 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: Apache-2.0

use crate::node_status_api::models::NymApiStorageError;
use thiserror::Error;
use validator_client::nymd::error::NymdError;
use validator_client::ValidatorClientError;

#[derive(Debug, Error)]
pub enum RewardingError {
    // #[error("There were no mixnodes to reward (network is dead)")]
    // NoMixnodesToReward,
    #[error("Failed to execute the smart contract - {0}")]
    ContractExecutionFailure(NymdError),

    // The inner error should be modified at some point...
    #[error("We run into storage issues - {0}")]
    StorageError(NymApiStorageError),

    #[error("Failed to query the smart contract - {0}")]
    ValidatorClientError(ValidatorClientError),

    #[error("Error downcasting u128 -> u64")]
    DowncastingError {
        #[from]
        source: std::num::TryFromIntError,
    },
    #[error("{source}")]
    WeightedError {
        #[from]
        source: rand::distributions::WeightedError,
    },
}

impl From<NymdError> for RewardingError {
    fn from(err: NymdError) -> Self {
        RewardingError::ContractExecutionFailure(err)
    }
}

impl From<NymApiStorageError> for RewardingError {
    fn from(err: NymApiStorageError) -> Self {
        RewardingError::StorageError(err)
    }
}

impl From<ValidatorClientError> for RewardingError {
    fn from(err: ValidatorClientError) -> Self {
        RewardingError::ValidatorClientError(err)
    }
}
