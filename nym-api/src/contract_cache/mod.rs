// Copyright 2021-2022 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: Apache-2.0

use crate::nymd_client::Client;
use ::time::OffsetDateTime;
use anyhow::Result;
use mixnet_contract_common::families::FamilyHead;
use mixnet_contract_common::{
    mixnode::MixNodeDetails, reward_params::RewardingParams, GatewayBond, IdentityKey, Interval,
    MixId, MixNodeBond, RewardedSetNodeStatus,
};
use nym_api_requests::models::MixnodeStatus;
use okapi::openapi3::OpenApi;
use rocket::fairing::AdHoc;
use rocket::Route;
use rocket_okapi::openapi_get_routes_spec;
use rocket_okapi::settings::OpenApiSettings;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use task::TaskClient;
use tokio::sync::{watch, RwLock};
use tokio::time;
use validator_client::nymd::CosmWasmClient;

pub(crate) mod routes;

// The cache can emit notifications to listeners about the current state
#[derive(Debug, PartialEq, Eq)]
pub enum CacheNotification {
    Start,
    Updated,
}

pub struct ValidatorCacheRefresher<C> {
    nymd_client: Client<C>,
    cache: ValidatorCache,
    caching_interval: Duration,

    // Notify listeners that the cache has been updated
    update_notifier: watch::Sender<CacheNotification>,
}

#[derive(Clone)]
pub struct ValidatorCache {
    initialised: Arc<AtomicBool>,
    inner: Arc<RwLock<ValidatorCacheInner>>,
}

struct ValidatorCacheInner {
    mixnodes: Cache<Vec<MixNodeDetails>>,
    gateways: Cache<Vec<GatewayBond>>,

    mixnodes_blacklist: Cache<HashSet<MixId>>,
    gateways_blacklist: Cache<HashSet<IdentityKey>>,

    rewarded_set: Cache<Vec<MixNodeDetails>>,
    active_set: Cache<Vec<MixNodeDetails>>,

    current_reward_params: Cache<Option<RewardingParams>>,
    current_interval: Cache<Option<Interval>>,

    mix_to_family: Cache<Vec<(IdentityKey, FamilyHead)>>,
}

fn current_unix_timestamp() -> i64 {
    let now = OffsetDateTime::now_utc();
    now.unix_timestamp()
}

#[derive(Default, Serialize, Clone)]
pub struct Cache<T> {
    value: T,
    as_at: i64,
}

impl<T: Clone> Cache<T> {
    pub(super) fn new(value: T) -> Self {
        Cache {
            value,
            as_at: current_unix_timestamp(),
        }
    }

    pub(super) fn update(&mut self, value: T) {
        self.value = value;
        self.as_at = current_unix_timestamp()
    }

    pub fn timestamp(&self) -> i64 {
        self.as_at
    }

    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> Deref for Cache<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<C> ValidatorCacheRefresher<C> {
    pub(crate) fn new(
        nymd_client: Client<C>,
        caching_interval: Duration,
        cache: ValidatorCache,
    ) -> Self {
        let (tx, _) = watch::channel(CacheNotification::Start);
        ValidatorCacheRefresher {
            nymd_client,
            cache,
            caching_interval,
            update_notifier: tx,
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<CacheNotification> {
        self.update_notifier.subscribe()
    }

    async fn get_rewarded_set_map(&self) -> HashMap<MixId, RewardedSetNodeStatus>
    where
        C: CosmWasmClient + Sync + Send,
    {
        self.nymd_client
            .get_rewarded_set_mixnodes()
            .await
            .map(|nodes| nodes.into_iter().collect())
            .unwrap_or_default()
    }

    fn collect_rewarded_and_active_set_details(
        all_mixnodes: &[MixNodeDetails],
        rewarded_set_nodes: &HashMap<MixId, RewardedSetNodeStatus>,
    ) -> (Vec<MixNodeDetails>, Vec<MixNodeDetails>) {
        let mut active_set = Vec::new();
        let mut rewarded_set = Vec::new();

        for mix in all_mixnodes {
            if let Some(status) = rewarded_set_nodes.get(&mix.mix_id()) {
                rewarded_set.push(mix.clone());
                if status.is_active() {
                    active_set.push(mix.clone())
                }
            }
        }

        (rewarded_set, active_set)
    }

    async fn refresh_cache(&self) -> Result<()>
    where
        C: CosmWasmClient + Sync + Send,
    {
        let rewarding_params = self.nymd_client.get_current_rewarding_parameters().await?;
        let current_interval = self.nymd_client.get_current_interval().await?.interval;

        let mixnodes = self.nymd_client.get_mixnodes().await?;
        let gateways = self.nymd_client.get_gateways().await?;

        let mix_to_family = self.nymd_client.get_all_family_members().await?;

        let rewarded_set_map = self.get_rewarded_set_map().await;

        let (rewarded_set, active_set) =
            Self::collect_rewarded_and_active_set_details(&mixnodes, &rewarded_set_map);

        info!(
            "Updating validator cache. There are {} mixnodes and {} gateways",
            mixnodes.len(),
            gateways.len(),
        );

        self.cache
            .update_cache(
                mixnodes,
                gateways,
                rewarded_set,
                active_set,
                rewarding_params,
                current_interval,
                mix_to_family,
            )
            .await;

        if let Err(err) = self.update_notifier.send(CacheNotification::Updated) {
            warn!("Failed to notify validator cache refresh: {err}");
        }

        Ok(())
    }

    pub(crate) async fn run(&self, mut shutdown: TaskClient)
    where
        C: CosmWasmClient + Sync + Send,
    {
        let mut interval = time::interval(self.caching_interval);
        while !shutdown.is_shutdown() {
            tokio::select! {
                _ = interval.tick() => {
                    tokio::select! {
                        biased;
                        _ = shutdown.recv() => {
                            trace!("ValidatorCacheRefresher: Received shutdown");
                        }
                        ret = self.refresh_cache() => {
                            if let Err(err) = ret {
                                error!("Failed to refresh validator cache - {err}");
                            } else {
                                // relaxed memory ordering is fine here. worst case scenario network monitor
                                // will just have to wait for an additional backoff to see the change.
                                // And so this will not really incur any performance penalties by setting it every loop iteration
                                self.cache.initialised.store(true, Ordering::Relaxed)
                            }
                        }
                    }
                }
                _ = shutdown.recv() => {
                    trace!("ValidatorCacheRefresher: Received shutdown");
                }
            }
        }
    }
}

pub(crate) fn validator_cache_routes(settings: &OpenApiSettings) -> (Vec<Route>, OpenApi) {
    openapi_get_routes_spec![
        settings: routes::get_mixnodes,
        routes::get_mixnodes_detailed,
        routes::get_gateways,
        routes::get_active_set,
        routes::get_active_set_detailed,
        routes::get_rewarded_set,
        routes::get_rewarded_set_detailed,
        routes::get_blacklisted_mixnodes,
        routes::get_blacklisted_gateways,
        routes::get_interval_reward_params,
        routes::get_current_epoch
    ]
}

impl ValidatorCache {
    fn new() -> Self {
        ValidatorCache {
            initialised: Arc::new(AtomicBool::new(false)),
            inner: Arc::new(RwLock::new(ValidatorCacheInner::new())),
        }
    }

    pub fn stage() -> AdHoc {
        AdHoc::on_ignite("Validator Cache Stage", |rocket| async {
            rocket.manage(Self::new())
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_cache(
        &self,
        mixnodes: Vec<MixNodeDetails>,
        gateways: Vec<GatewayBond>,
        rewarded_set: Vec<MixNodeDetails>,
        active_set: Vec<MixNodeDetails>,
        rewarding_params: RewardingParams,
        current_interval: Interval,
        mix_to_family: Vec<(IdentityKey, FamilyHead)>,
    ) {
        match time::timeout(Duration::from_millis(100), self.inner.write()).await {
            Ok(mut cache) => {
                cache.mixnodes.update(mixnodes);
                cache.gateways.update(gateways);
                cache.rewarded_set.update(rewarded_set);
                cache.active_set.update(active_set);
                cache.current_reward_params.update(Some(rewarding_params));
                cache.current_interval.update(Some(current_interval));
                cache.mix_to_family.update(mix_to_family)
            }
            Err(e) => {
                error!("{}", e);
            }
        }
    }

    pub async fn mixnodes_blacklist(&self) -> Option<Cache<HashSet<MixId>>> {
        match time::timeout(Duration::from_millis(100), self.inner.read()).await {
            Ok(cache) => Some(cache.mixnodes_blacklist.clone()),
            Err(e) => {
                error!("{}", e);
                None
            }
        }
    }

    pub async fn gateways_blacklist(&self) -> Option<Cache<HashSet<IdentityKey>>> {
        match time::timeout(Duration::from_millis(100), self.inner.read()).await {
            Ok(cache) => Some(cache.gateways_blacklist.clone()),
            Err(e) => {
                error!("{}", e);
                None
            }
        }
    }

    pub async fn update_mixnodes_blacklist(&self, add: HashSet<MixId>, remove: HashSet<MixId>) {
        let blacklist = self.mixnodes_blacklist().await;
        if let Some(blacklist) = blacklist {
            let mut blacklist = blacklist
                .value
                .union(&add)
                .cloned()
                .collect::<HashSet<MixId>>();
            let to_remove = blacklist
                .intersection(&remove)
                .cloned()
                .collect::<HashSet<MixId>>();
            for key in to_remove {
                blacklist.remove(&key);
            }
            match time::timeout(Duration::from_millis(100), self.inner.write()).await {
                Ok(mut cache) => {
                    cache.mixnodes_blacklist.update(blacklist);
                    return;
                }
                Err(e) => error!("{}", e),
            }
        }
        error!("Failed to update mixnodes blacklist");
    }

    pub async fn update_gateways_blacklist(
        &self,
        add: HashSet<IdentityKey>,
        remove: HashSet<IdentityKey>,
    ) {
        let blacklist = self.gateways_blacklist().await;
        if let Some(blacklist) = blacklist {
            let mut blacklist = blacklist
                .value
                .union(&add)
                .cloned()
                .collect::<HashSet<IdentityKey>>();
            let to_remove = blacklist
                .intersection(&remove)
                .cloned()
                .collect::<HashSet<IdentityKey>>();
            for key in to_remove {
                blacklist.remove(&key);
            }
            match time::timeout(Duration::from_millis(100), self.inner.write()).await {
                Ok(mut cache) => {
                    cache.gateways_blacklist.update(blacklist);
                    return;
                }
                Err(e) => error!("{}", e),
            }
        }
        error!("Failed to update gateways blacklist");
    }

    pub async fn mixnodes(&self) -> Vec<MixNodeDetails> {
        let blacklist = self.mixnodes_blacklist().await;
        let mixnodes = match time::timeout(Duration::from_millis(100), self.inner.read()).await {
            Ok(cache) => cache.mixnodes.clone(),
            Err(e) => {
                error!("{}", e);
                return Vec::new();
            }
        };

        if let Some(blacklist) = blacklist {
            mixnodes
                .value
                .iter()
                .filter(|mix| !blacklist.value.contains(&mix.mix_id()))
                .cloned()
                .collect()
        } else {
            mixnodes.value
        }
    }

    pub async fn mixnodes_basic(&self) -> Vec<MixNodeBond> {
        match time::timeout(Duration::from_millis(100), self.inner.read()).await {
            Ok(cache) => cache
                .mixnodes
                .clone()
                .into_inner()
                .into_iter()
                .map(|bond| bond.bond_information)
                .collect(),
            Err(e) => {
                error!("{}", e);
                Vec::new()
            }
        }
    }

    pub async fn gateways(&self) -> Vec<GatewayBond> {
        let blacklist = self.gateways_blacklist().await;
        let gateways = match time::timeout(Duration::from_millis(100), self.inner.read()).await {
            Ok(cache) => cache.gateways.clone(),
            Err(e) => {
                error!("{}", e);
                return Vec::new();
            }
        };

        if let Some(blacklist) = blacklist {
            gateways
                .value
                .iter()
                .filter(|mix| !blacklist.value.contains(mix.identity()))
                .cloned()
                .collect()
        } else {
            gateways.value
        }
    }

    pub async fn gateways_all(&self) -> Vec<GatewayBond> {
        match time::timeout(Duration::from_millis(100), self.inner.read()).await {
            Ok(cache) => cache.gateways.value.clone(),
            Err(e) => {
                error!("{}", e);
                Vec::new()
            }
        }
    }

    pub async fn rewarded_set(&self) -> Cache<Vec<MixNodeDetails>> {
        match time::timeout(Duration::from_millis(100), self.inner.read()).await {
            Ok(cache) => cache.rewarded_set.clone(),
            Err(e) => {
                error!("{}", e);
                Cache::new(Vec::new())
            }
        }
    }

    pub async fn active_set(&self) -> Cache<Vec<MixNodeDetails>> {
        match time::timeout(Duration::from_millis(100), self.inner.read()).await {
            Ok(cache) => cache.active_set.clone(),
            Err(e) => {
                error!("{}", e);
                Cache::new(Vec::new())
            }
        }
    }

    pub async fn mix_to_family(&self) -> Cache<Vec<(IdentityKey, FamilyHead)>> {
        match time::timeout(Duration::from_millis(100), self.inner.read()).await {
            Ok(cache) => cache.mix_to_family.clone(),
            Err(e) => {
                error!("{}", e);
                Cache::new(Vec::new())
            }
        }
    }

    pub(crate) async fn interval_reward_params(&self) -> Cache<Option<RewardingParams>> {
        match time::timeout(Duration::from_millis(100), self.inner.read()).await {
            Ok(cache) => cache.current_reward_params.clone(),
            Err(e) => {
                error!("{}", e);
                Cache::new(None)
            }
        }
    }

    pub(crate) async fn current_interval(&self) -> Cache<Option<Interval>> {
        match time::timeout(Duration::from_millis(100), self.inner.read()).await {
            Ok(cache) => cache.current_interval.clone(),
            Err(e) => {
                error!("{}", e);
                Cache::new(None)
            }
        }
    }

    pub async fn mixnode_details(&self, mix_id: MixId) -> (Option<MixNodeDetails>, MixnodeStatus) {
        // it might not be the most optimal to possibly iterate the entire vector to find (or not)
        // the relevant value. However, the vectors are relatively small (< 10_000 elements, < 1000 for active set)

        let active_set = &self.active_set().await.value;
        if let Some(bond) = active_set.iter().find(|mix| mix.mix_id() == mix_id) {
            return (Some(bond.clone()), MixnodeStatus::Active);
        }

        let rewarded_set = &self.rewarded_set().await.value;
        if let Some(bond) = rewarded_set.iter().find(|mix| mix.mix_id() == mix_id) {
            return (Some(bond.clone()), MixnodeStatus::Standby);
        }

        let all_bonded = &self.mixnodes().await;
        if let Some(bond) = all_bonded.iter().find(|mix| mix.mix_id() == mix_id) {
            (Some(bond.clone()), MixnodeStatus::Inactive)
        } else {
            (None, MixnodeStatus::NotFound)
        }
    }

    pub async fn mixnode_status(&self, mix_id: MixId) -> MixnodeStatus {
        self.mixnode_details(mix_id).await.1
    }

    pub fn initialised(&self) -> bool {
        self.initialised.load(Ordering::Relaxed)
    }

    pub(crate) async fn wait_for_initial_values(&self) {
        let initialisation_backoff = Duration::from_secs(5);
        loop {
            if self.initialised() {
                break;
            } else {
                debug!("Validator cache hasn't been initialised yet - waiting for {:?} before trying again", initialisation_backoff);
                tokio::time::sleep(initialisation_backoff).await;
            }
        }
    }
}

impl ValidatorCacheInner {
    fn new() -> Self {
        ValidatorCacheInner {
            mixnodes: Cache::default(),
            gateways: Cache::default(),
            rewarded_set: Cache::default(),
            active_set: Cache::default(),
            mixnodes_blacklist: Cache::default(),
            gateways_blacklist: Cache::default(),
            current_interval: Cache::default(),
            current_reward_params: Cache::default(),
            mix_to_family: Cache::default(),
        }
    }
}
