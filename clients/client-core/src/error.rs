// Copyright 2022 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: Apache-2.0

use crate::client::replies::reply_storage::ReplyStorageBackend;
use crypto::asymmetric::identity::Ed25519RecoveryError;
use gateway_client::error::GatewayClientError;
use topology::NymTopologyError;
use validator_client::ValidatorClientError;

#[derive(thiserror::Error, Debug)]
pub enum ClientCoreError<B: ReplyStorageBackend> {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Gateway client error: {0}")]
    GatewayClientError(#[from] GatewayClientError),

    #[error("Ed25519 error: {0}")]
    Ed25519RecoveryError(#[from] Ed25519RecoveryError),

    #[error("Validator client error: {0}")]
    ValidatorClientError(#[from] ValidatorClientError),

    #[error("No gateway with id: {0}")]
    NoGatewayWithId(String),

    #[error("No gateways on network")]
    NoGatewaysOnNetwork,

    #[error("Failed to setup gateway")]
    FailedToSetupGateway,

    #[error("List of nym apis is empty")]
    ListOfNymApisIsEmpty,

    #[error("Could not load existing gateway configuration: {0}")]
    CouldNotLoadExistingGatewayConfiguration(std::io::Error),

    #[error("The current network topology seem to be insufficient to route any packets through")]
    InsufficientNetworkTopology(#[from] NymTopologyError),

    #[error("experienced a failure with our reply surb persistent storage: {source}")]
    SurbStorageError { source: B::StorageError },

    #[error("The gateway id is invalid - {0}")]
    UnableToCreatePublicKeyFromGatewayId(Ed25519RecoveryError),

    #[error("The identity of the gateway is unknwown - did you run init?")]
    GatewayIdUnknown,

    #[error("The owner of the gateway is unknown - did you run init?")]
    GatewayOwnerUnknown,

    #[error("The address of the gateway is unknown - did you run init?")]
    GatwayAddressUnknown,

    #[error("Unexpected exit")]
    UnexpectedExit,
}

/// Set of messages that the client can send to listeners via the task manager
#[derive(thiserror::Error, Debug)]
pub enum ClientCoreStatusMessage {
    #[error("The connected gateway is slow, or the connection to it is slow")]
    GatewayIsSlow,
    #[error("The connected gateway is very slow, or the connection to it is very slow")]
    GatewayIsVerySlow,
}
