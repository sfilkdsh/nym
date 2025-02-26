// Copyright 2021 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: Apache-2.0

use std::error::Error;

use crate::{
    client::{config::Config, SocketClient},
    commands::{override_config, OverrideConfig},
    error::ClientError,
};

use clap::Args;
use config::NymConfig;
use log::*;
use version_checker::is_minor_version_compatible;

#[derive(Args, Clone)]
pub(crate) struct Run {
    /// Id of the nym-mixnet-client we want to run.
    #[clap(long)]
    id: String,

    /// Comma separated list of rest endpoints of the nymd validators
    #[clap(long)]
    nymd_validators: Option<String>,

    /// Comma separated list of rest endpoints of the API validators
    #[clap(long)]
    api_validators: Option<String>,

    /// Id of the gateway we want to connect to. If overridden, it is user's responsibility to
    /// ensure prior registration happened
    #[clap(long)]
    gateway: Option<String>,

    /// Whether to not start the websocket
    #[clap(long)]
    disable_socket: bool,

    /// Port for the socket to listen on
    #[clap(short, long)]
    port: Option<u16>,

    /// Mostly debug-related option to increase default traffic rate so that you would not need to
    /// modify config post init
    #[clap(long, hidden = true)]
    fastmode: bool,

    /// Disable loop cover traffic and the Poisson rate limiter (for debugging only)
    #[clap(long, hidden = true)]
    no_cover: bool,

    /// Set this client to work in a enabled credentials mode that would attempt to use gateway
    /// with bandwidth credential requirement.
    #[cfg(feature = "coconut")]
    #[clap(long)]
    enabled_credentials_mode: bool,
}

impl From<Run> for OverrideConfig {
    fn from(run_config: Run) -> Self {
        OverrideConfig {
            nymd_validators: run_config.nymd_validators,
            api_validators: run_config.api_validators,
            disable_socket: run_config.disable_socket,
            port: run_config.port,
            fastmode: run_config.fastmode,
            no_cover: run_config.no_cover,
            #[cfg(feature = "coconut")]
            enabled_credentials_mode: run_config.enabled_credentials_mode,
        }
    }
}

// this only checks compatibility between config the binary. It does not take into consideration
// network version. It might do so in the future.
fn version_check(cfg: &Config) -> bool {
    let binary_version = env!("CARGO_PKG_VERSION");
    let config_version = cfg.get_base().get_version();
    if binary_version == config_version {
        true
    } else {
        warn!("The native-client binary has different version than what is specified in config file! {} and {}", binary_version, config_version);
        if is_minor_version_compatible(binary_version, config_version) {
            info!("but they are still semver compatible. However, consider running the `upgrade` command");
            true
        } else {
            error!("and they are semver incompatible! - please run the `upgrade` command before attempting `run` again");
            false
        }
    }
}

pub(crate) async fn execute(args: &Run) -> Result<(), Box<dyn Error + Send + Sync>> {
    let id = &args.id;

    let mut config = match Config::load_from_file(Some(id)) {
        Ok(cfg) => cfg,
        Err(err) => {
            error!("Failed to load config for {}. Are you sure you have run `init` before? (Error was: {err})", id);
            return Err(Box::new(ClientError::FailedToLoadConfig(id.to_string())));
        }
    };

    let override_config_fields = OverrideConfig::from(args.clone());
    config = override_config(config, override_config_fields);

    if config.get_base_mut().set_empty_fields_to_defaults() {
        warn!("some of the core config options were left unset. the default values are going to get used instead.");
    }

    if !version_check(&config) {
        error!("failed the local version check");
        return Err(Box::new(ClientError::FailedLocalVersionCheck));
    }

    SocketClient::new(config).run_socket_forever().await
}
