// Copyright 2022 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: Apache-2.0

use crate::client::replies::reply_storage::CombinedReplyStorage;
use async_trait::async_trait;
use std::error::Error;
use thiserror::Error;

#[cfg(target_arch = "wasm32")]
pub mod browser_backend;

#[cfg(all(not(target_arch = "wasm32"), feature = "fs-surb-storage"))]
pub mod fs_backend;

// #[cfg(all(test, feature = "std"))]
// third case: node with actual filesystem

#[derive(Debug, Error)]
#[error("no information provided")]
pub struct UndefinedError;

pub struct Empty {
    // we need to keep 'basic' metadata here to "load" the CombinedReplyStorage
    min_surb_threshold: usize,
    max_surb_threshold: usize,
}

#[async_trait]
impl ReplyStorageBackend for Empty {
    type StorageError = UndefinedError;

    async fn flush_surb_storage(
        &mut self,
        _storage: &CombinedReplyStorage,
    ) -> Result<(), Self::StorageError> {
        Ok(())
    }

    async fn init_fresh(
        &mut self,
        _fresh: &CombinedReplyStorage,
    ) -> Result<(), Self::StorageError> {
        Ok(())
    }

    async fn load_surb_storage(&self) -> Result<CombinedReplyStorage, Self::StorageError> {
        Ok(CombinedReplyStorage::new(
            self.min_surb_threshold,
            self.max_surb_threshold,
        ))
    }
}

#[async_trait]
pub trait ReplyStorageBackend: Sized {
    type StorageError: Error + 'static;

    async fn start_storage_session(&self) -> Result<(), Self::StorageError> {
        Ok(())
    }

    // reply keys and surbs would need additional field set when data is loaded
    // so if there's some failure, we'd trash it all
    async fn flush_surb_storage(
        &mut self,
        storage: &CombinedReplyStorage,
    ) -> Result<(), Self::StorageError>;

    /// The purpose of this call is to save any metadata that might be present.
    /// (such as surb thresholds)
    async fn init_fresh(&mut self, fresh: &CombinedReplyStorage) -> Result<(), Self::StorageError>;

    async fn load_surb_storage(&self) -> Result<CombinedReplyStorage, Self::StorageError>;

    async fn stop_storage_session(self) -> Result<(), Self::StorageError> {
        Ok(())
    }
}
