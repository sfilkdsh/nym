// Copyright 2020 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: Apache-2.0

use crate::node::client_handling::active_clients::ActiveClientsStore;
use crate::node::client_handling::websocket::connection_handler::FreshHandler;
use crate::node::storage::Storage;
use crypto::asymmetric::identity;
use log::*;
use mixnet_client::forwarder::MixForwardingSender;
use rand::rngs::OsRng;
use std::net::SocketAddr;
use std::process;
use std::sync::Arc;
use tokio::task::JoinHandle;

#[cfg(feature = "coconut")]
use crate::node::client_handling::websocket::connection_handler::coconut::CoconutVerifier;

pub(crate) struct Listener {
    address: SocketAddr,
    local_identity: Arc<identity::KeyPair>,
    only_coconut_credentials: bool,

    #[cfg(feature = "coconut")]
    pub(crate) coconut_verifier: Arc<CoconutVerifier>,
}

impl Listener {
    pub(crate) fn new(
        address: SocketAddr,
        local_identity: Arc<identity::KeyPair>,
        only_coconut_credentials: bool,
        #[cfg(feature = "coconut")] coconut_verifier: Arc<CoconutVerifier>,
    ) -> Self {
        Listener {
            address,
            local_identity,
            only_coconut_credentials,
            #[cfg(feature = "coconut")]
            coconut_verifier,
        }
    }

    // TODO: change the signature to pub(crate) async fn run(&self, handler: Handler)

    pub(crate) async fn run<St>(
        &mut self,
        outbound_mix_sender: MixForwardingSender,
        storage: St,
        active_clients_store: ActiveClientsStore,
    ) where
        St: Storage + Clone + 'static,
    {
        info!("Starting websocket listener at {}", self.address);
        let tcp_listener = match tokio::net::TcpListener::bind(self.address).await {
            Ok(listener) => listener,
            Err(err) => {
                error!("Failed to bind the websocket to {} - {err}. Are you sure nothing else is running on the specified port and your user has sufficient permission to bind to the requested address?", self.address);
                process::exit(1);
            }
        };

        loop {
            match tcp_listener.accept().await {
                Ok((socket, remote_addr)) => {
                    trace!("received a socket connection from {}", remote_addr);
                    // TODO: I think we *REALLY* need a mechanism for having a maximum number of connected
                    // clients or spawned tokio tasks -> perhaps a worker system?
                    let handle = FreshHandler::new(
                        OsRng,
                        socket,
                        self.only_coconut_credentials,
                        outbound_mix_sender.clone(),
                        Arc::clone(&self.local_identity),
                        storage.clone(),
                        active_clients_store.clone(),
                        #[cfg(feature = "coconut")]
                        Arc::clone(&self.coconut_verifier),
                    );
                    tokio::spawn(async move { handle.start_handling().await });
                }
                Err(err) => warn!("failed to get client: {err}"),
            }
        }
    }

    pub(crate) fn start<St>(
        mut self,
        outbound_mix_sender: MixForwardingSender,
        storage: St,
        active_clients_store: ActiveClientsStore,
    ) -> JoinHandle<()>
    where
        St: Storage + Clone + 'static,
    {
        tokio::spawn(async move {
            self.run(outbound_mix_sender, storage, active_clients_store)
                .await
        })
    }
}
