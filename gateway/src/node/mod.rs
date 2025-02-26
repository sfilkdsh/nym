// Copyright 2020 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: Apache-2.0

use crate::commands::sign::load_identity_keys;
use crate::commands::validate_bech32_address_or_exit;
use crate::config::Config;
use crate::node::client_handling::active_clients::ActiveClientsStore;
use crate::node::client_handling::websocket;
use crate::node::mixnet_handling::receiver::connection_handler::ConnectionHandler;
use crate::node::statistics::collector::GatewayStatisticsCollector;
use crate::node::storage::Storage;
use crypto::asymmetric::{encryption, identity};
use log::*;
use mixnet_client::forwarder::{MixForwardingSender, PacketForwarder};
#[cfg(feature = "coconut")]
use network_defaults::NymNetworkDetails;
use rand::seq::SliceRandom;
use rand::thread_rng;
use statistics_common::collector::StatisticsSender;
use std::net::SocketAddr;
use std::process;
use std::sync::Arc;

use crate::config::persistence::pathfinder::GatewayPathfinder;
#[cfg(feature = "coconut")]
use crate::node::client_handling::websocket::connection_handler::coconut::CoconutVerifier;
#[cfg(feature = "coconut")]
use credentials::coconut::utils::obtain_aggregate_verification_key;
#[cfg(feature = "coconut")]
use validator_client::{Client, CoconutApiClient};

use self::storage::PersistentStorage;

pub(crate) mod client_handling;
pub(crate) mod mixnet_handling;
pub(crate) mod statistics;
pub(crate) mod storage;

/// Wire up and create Gateway instance
pub(crate) async fn create_gateway(config: Config) -> Gateway<PersistentStorage> {
    let storage = initialise_storage(&config).await;
    Gateway::new(config, storage).await
}

async fn initialise_storage(config: &Config) -> PersistentStorage {
    let path = config.get_persistent_store_path();
    let retrieval_limit = config.get_message_retrieval_limit();
    match PersistentStorage::init(path, retrieval_limit).await {
        Err(err) => panic!("failed to initialise gateway storage - {err}"),
        Ok(storage) => storage,
    }
}

pub(crate) struct Gateway<St: Storage> {
    config: Config,
    /// ed25519 keypair used to assert one's identity.
    identity_keypair: Arc<identity::KeyPair>,
    /// x25519 keypair used for Diffie-Hellman. Currently only used for sphinx key derivation.
    sphinx_keypair: Arc<encryption::KeyPair>,
    storage: St,
}

impl<St> Gateway<St>
where
    St: Storage + Clone + 'static,
{
    /// Construct from the given `Config` instance.
    pub async fn new(config: Config, storage: St) -> Self {
        let pathfinder = GatewayPathfinder::new_from_config(&config);
        // let storage = Self::initialise_storage(&config).await;

        Gateway {
            config,
            identity_keypair: Arc::new(Self::load_identity_keys(&pathfinder)),
            sphinx_keypair: Arc::new(Self::load_sphinx_keys(&pathfinder)),
            storage,
        }
    }

    #[cfg(test)]
    pub async fn new_from_keys_and_storage(
        config: Config,
        identity_keypair: identity::KeyPair,
        sphinx_keypair: encryption::KeyPair,
        storage: St,
    ) -> Self {
        Gateway {
            config,
            identity_keypair: Arc::new(identity_keypair),
            sphinx_keypair: Arc::new(sphinx_keypair),
            storage,
        }
    }

    fn load_identity_keys(pathfinder: &GatewayPathfinder) -> identity::KeyPair {
        let identity_keypair: identity::KeyPair =
            pemstore::load_keypair(&pemstore::KeyPairPath::new(
                pathfinder.private_identity_key().to_owned(),
                pathfinder.public_identity_key().to_owned(),
            ))
            .expect("Failed to read stored identity key files");
        identity_keypair
    }

    fn load_sphinx_keys(pathfinder: &GatewayPathfinder) -> encryption::KeyPair {
        let sphinx_keypair: encryption::KeyPair =
            pemstore::load_keypair(&pemstore::KeyPairPath::new(
                pathfinder.private_encryption_key().to_owned(),
                pathfinder.public_encryption_key().to_owned(),
            ))
            .expect("Failed to read stored sphinx key files");
        sphinx_keypair
    }

    /// Signs the node config's bech32 address to produce a verification code for use in the wallet.
    /// Exits if the address isn't valid (which should protect against manual edits).
    fn generate_owner_signature(&self) -> String {
        let pathfinder = GatewayPathfinder::new_from_config(&self.config);
        let identity_keypair = load_identity_keys(&pathfinder);
        let address = self.config.get_wallet_address();
        validate_bech32_address_or_exit(address);
        let verification_code = identity_keypair.private_key().sign_text(address);
        verification_code
    }

    pub(crate) fn print_node_details(&self) {
        println!(
            "Identity Key: {}",
            self.identity_keypair.public_key().to_base58_string()
        );
        println!(
            "Sphinx Key: {}",
            self.sphinx_keypair.public_key().to_base58_string()
        );
        println!("Owner Signature: {}", self.generate_owner_signature());
        println!(
            "Host: {} (bind address: {})",
            self.config.get_announce_address(),
            self.config.get_listening_address()
        );
        println!("Version: {}", self.config.get_version());
        println!(
            "Mix Port: {}, Clients port: {}",
            self.config.get_mix_port(),
            self.config.get_clients_port()
        );

        println!(
            "Data store is at: {:?}",
            self.config.get_persistent_store_path()
        );
    }

    fn start_mix_socket_listener(
        &self,
        ack_sender: MixForwardingSender,
        active_clients_store: ActiveClientsStore,
    ) {
        info!("Starting mix socket listener...");

        let packet_processor =
            mixnet_handling::PacketProcessor::new(self.sphinx_keypair.private_key());

        let connection_handler = ConnectionHandler::new(
            packet_processor,
            self.storage.clone(),
            ack_sender,
            active_clients_store,
        );

        let listening_address = SocketAddr::new(
            self.config.get_listening_address(),
            self.config.get_mix_port(),
        );

        mixnet_handling::Listener::new(listening_address).start(connection_handler);
    }

    fn start_client_websocket_listener(
        &self,
        forwarding_channel: MixForwardingSender,
        active_clients_store: ActiveClientsStore,
        #[cfg(feature = "coconut")] coconut_verifier: Arc<CoconutVerifier>,
    ) {
        info!("Starting client [web]socket listener...");

        let listening_address = SocketAddr::new(
            self.config.get_listening_address(),
            self.config.get_clients_port(),
        );

        websocket::Listener::new(
            listening_address,
            Arc::clone(&self.identity_keypair),
            self.config.get_only_coconut_credentials(),
            #[cfg(feature = "coconut")]
            coconut_verifier,
        )
        .start(
            forwarding_channel,
            self.storage.clone(),
            active_clients_store,
        );
    }

    fn start_packet_forwarder(&self) -> MixForwardingSender {
        info!("Starting mix packet forwarder...");

        let (mut packet_forwarder, packet_sender) = PacketForwarder::new(
            self.config.get_packet_forwarding_initial_backoff(),
            self.config.get_packet_forwarding_maximum_backoff(),
            self.config.get_initial_connection_timeout(),
            self.config.get_maximum_connection_buffer_size(),
            self.config.get_use_legacy_sphinx_framing(),
        );

        tokio::spawn(async move { packet_forwarder.run().await });
        packet_sender
    }

    async fn wait_for_interrupt(&self) {
        if let Err(e) = tokio::signal::ctrl_c().await {
            error!(
                "There was an error while capturing SIGINT - {:?}. We will terminate regardless",
                e
            );
        }
        println!(
            "Received SIGINT - the gateway will terminate now (threads are not yet nicely stopped, if you see stack traces that's alright)."
        );
    }

    fn random_api_client(&self) -> validator_client::ApiClient {
        let endpoints = self.config.get_nym_api_endpoints();
        let nym_api = endpoints
            .choose(&mut thread_rng())
            .expect("The list of validator apis is empty");

        validator_client::ApiClient::new(nym_api.clone())
    }

    #[cfg(feature = "coconut")]
    fn random_nymd_client(
        &self,
    ) -> validator_client::Client<validator_client::nymd::SigningNymdClient> {
        let endpoints = self.config.get_validator_nymd_endpoints();
        let validator_nymd = endpoints
            .choose(&mut thread_rng())
            .expect("The list of validators is empty");

        let network_details = NymNetworkDetails::new_from_env();
        let client_config = validator_client::Config::try_from_nym_network_details(
            &network_details,
        )
        .expect("failed to construct valid validator client config with the provided network");

        let mut client = Client::new_signing(client_config, self.config.get_cosmos_mnemonic())
            .expect("Could not connect with mnemonic");
        client
            .change_nymd(validator_nymd.clone())
            .expect("Could not use the random nymd URL");
        client
    }

    // TODO: ask DH whether this function still makes sense in ^0.10
    async fn check_if_same_ip_gateway_exists(&self) -> Option<String> {
        let validator_client = self.random_api_client();

        let existing_gateways = match validator_client.get_cached_gateways().await {
            Ok(gateways) => gateways,
            Err(err) => {
                error!("failed to grab initial network gateways - {err}\n Please try to startup again in few minutes");
                process::exit(1);
            }
        };

        let our_host = self.config.get_announce_address();

        existing_gateways
            .iter()
            .find(|node| node.gateway.host == our_host)
            .map(|node| node.gateway().identity_key.clone())
    }

    pub async fn run(&mut self) {
        info!("Starting nym gateway!");

        if let Some(duplicate_node_key) = self.check_if_same_ip_gateway_exists().await {
            if duplicate_node_key == self.identity_keypair.public_key().to_base58_string() {
                warn!("We seem to have not unregistered after going offline - there's a node with identical identity and announce-host as us registered.")
            } else {
                error!(
                    "Our announce-host is identical to an existing node's announce-host! (its key is {:?})",
                    duplicate_node_key
                );
                return;
            }
        }

        #[cfg(feature = "coconut")]
        let coconut_verifier = {
            let nymd_client = self.random_nymd_client();
            let api_clients = CoconutApiClient::all_coconut_api_clients(&nymd_client)
                .await
                .expect("Could not query all api clients");
            let validators_verification_key = obtain_aggregate_verification_key(&api_clients)
                .await
                .expect("failed to contact validators to obtain their verification keys");
            CoconutVerifier::new(
                api_clients,
                nymd_client,
                std::env::var(network_defaults::var_names::MIX_DENOM)
                    .expect("mix denom base not set"),
                validators_verification_key,
            )
            .expect("Could not create coconut verifier")
        };

        let mix_forwarding_channel = self.start_packet_forwarder();

        let active_clients_store = ActiveClientsStore::new();
        self.start_mix_socket_listener(
            mix_forwarding_channel.clone(),
            active_clients_store.clone(),
        );

        if self.config.get_enabled_statistics() {
            let statistics_service_url = self.config.get_statistics_service_url();
            let stats_collector = GatewayStatisticsCollector::new(
                self.identity_keypair.public_key().to_base58_string(),
                active_clients_store.clone(),
                statistics_service_url,
            );
            let mut stats_sender = StatisticsSender::new(stats_collector);
            tokio::spawn(async move {
                stats_sender.run().await;
            });
        }

        self.start_client_websocket_listener(
            mix_forwarding_channel,
            active_clients_store,
            #[cfg(feature = "coconut")]
            Arc::new(coconut_verifier),
        );

        info!("Finished nym gateway startup procedure - it should now be able to receive mix and client traffic!");

        self.wait_for_interrupt().await
    }
}
