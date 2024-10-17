//! Database access for `eth_` block RPC methods. Loads block and receipt data w.r.t. network.

use std::sync::Arc;

use alloy_rpc_types::{Header, Index};
use futures::Future;
use reth_primitives::{BlockId, Receipt, SealedBlock, SealedBlockWithSenders};
use reth_provider::{BlockIdReader, BlockReader, BlockReaderIdExt, HeaderProvider};
use reth_rpc_eth_types::EthStateCache;
use reth_rpc_types_compat::block::{from_block, uncle_block_from_header};

use crate::{FromEthApiError, FullEthApiTypes, RpcBlock, RpcReceipt};

use super::{LoadPendingBlock, LoadReceipt, SpawnBlocking};

/// Result type of the fetched block receipts.
pub type BlockReceiptsResult<N, E> = Result<Option<Vec<RpcReceipt<N>>>, E>;
/// Result type of the fetched block and its receipts.
pub type BlockAndReceiptsResult<E> = Result<Option<(SealedBlock, Arc<Vec<Receipt>>)>, E>;

/// Block related functions for the [`EthApiServer`](crate::EthApiServer) trait in the
/// `eth_` namespace.
pub trait EthBlocks: LoadBlock {
    /// Returns a handle for reading data from disk.
    ///
    /// Data access in default (L1) trait method implementations.
    fn provider(&self) -> impl HeaderProvider;

    /// Returns the block header for the given block id.
    fn rpc_block_header(
        &self,
        block_id: BlockId,
    ) -> impl Future<Output = Result<Option<Header>, Self::Error>> + Send
    where
        Self: FullEthApiTypes,
    {
        async move { Ok(self.rpc_block(block_id, false).await?.map(|block| block.header)) }
    }

    /// Returns the populated rpc block object for the given block id.
    ///
    /// If `full` is true, the block object will contain all transaction objects, otherwise it will
    /// only contain the transaction hashes.
    fn rpc_block(
        &self,
        block_id: BlockId,
        full: bool,
    ) -> impl Future<Output = Result<Option<RpcBlock<Self::NetworkTypes>>, Self::Error>> + Send
    where
        Self: FullEthApiTypes,
    {
        async move {
            let Some(block) = self.block_with_senders(block_id).await? else { return Ok(None) };
            let block_hash = block.hash();
            let mut total_difficulty = EthBlocks::provider(self)
                .header_td_by_number(block.number)
                .map_err(Self::Error::from_eth_err)?;
            if total_difficulty.is_none() {
                // if we failed to find td after we successfully loaded the block, try again using
                // the hash this only matters if the chain is currently transitioning the merge block and there's a reorg: <https://github.com/paradigmxyz/reth/issues/10941>
                total_difficulty = EthBlocks::provider(self)
                    .header_td(&block.hash())
                    .map_err(Self::Error::from_eth_err)?;
            }

            let block = from_block::<Self::TransactionCompat>(
                (*block).clone().unseal(),
                total_difficulty.unwrap_or_default(),
                full.into(),
                Some(block_hash),
            )
            .map_err(Self::Error::from_eth_err)?;
            Ok(Some(block))
        }
    }

    /// Returns the number transactions in the given block.
    ///
    /// Returns `None` if the block does not exist
    fn block_transaction_count(
        &self,
        block_id: BlockId,
    ) -> impl Future<Output = Result<Option<usize>, Self::Error>> + Send {
        async move {
            if block_id.is_pending() {
                // Pending block can be fetched directly without need for caching
                return Ok(LoadBlock::provider(self)
                    .pending_block()
                    .map_err(Self::Error::from_eth_err)?
                    .map(|block| block.body.transactions.len()))
            }

            let block_hash = match LoadBlock::provider(self)
                .block_hash_for_id(block_id)
                .map_err(Self::Error::from_eth_err)?
            {
                Some(block_hash) => block_hash,
                None => return Ok(None),
            };

            Ok(self
                .cache()
                .get_sealed_block_with_senders(block_hash)
                .await
                .map_err(Self::Error::from_eth_err)?
                .map(|b| b.body.transactions.len()))
        }
    }

    /// Helper function for `eth_getBlockReceipts`.
    ///
    /// Returns all transaction receipts in block, or `None` if block wasn't found.
    #[allow(clippy::type_complexity)]
    fn block_receipts(
        &self,
        block_id: BlockId,
    ) -> impl Future<Output = BlockReceiptsResult<Self::NetworkTypes, Self::Error>> + Send
    where
        Self: LoadReceipt;

    /// Helper method that loads a bock and all its receipts.
    #[allow(clippy::type_complexity)]
    fn load_block_and_receipts(
        &self,
        block_id: BlockId,
    ) -> impl Future<Output = BlockAndReceiptsResult<Self::Error>> + Send
    where
        Self: LoadReceipt,
    {
        async move {
            if block_id.is_pending() {
                // First, try to get the pending block from the provider, in case we already
                // received the actual pending block from the CL.
                if let Some((block, receipts)) = LoadBlock::provider(self)
                    .pending_block_and_receipts()
                    .map_err(Self::Error::from_eth_err)?
                {
                    return Ok(Some((block, Arc::new(receipts))));
                }

                // If no pending block from provider, build the pending block locally.
                if let Some((block, receipts)) = self.local_pending_block().await? {
                    return Ok(Some((block.block, Arc::new(receipts))));
                }
            }

            if let Some(block_hash) = LoadBlock::provider(self)
                .block_hash_for_id(block_id)
                .map_err(Self::Error::from_eth_err)?
            {
                return LoadReceipt::cache(self)
                    .get_block_and_receipts(block_hash)
                    .await
                    .map_err(Self::Error::from_eth_err)
                    .map(|b| b.map(|(b, r)| (b.block.clone(), r)))
            }

            Ok(None)
        }
    }

    /// Returns uncle headers of given block.
    ///
    /// Returns an empty vec if there are none.
    fn ommers(
        &self,
        block_id: BlockId,
    ) -> Result<Option<Vec<reth_primitives::Header>>, Self::Error> {
        LoadBlock::provider(self).ommers_by_id(block_id).map_err(Self::Error::from_eth_err)
    }

    /// Returns uncle block at given index in given block.
    ///
    /// Returns `None` if index out of range.
    fn ommer_by_block_and_index(
        &self,
        block_id: BlockId,
        index: Index,
    ) -> impl Future<Output = Result<Option<RpcBlock<Self::NetworkTypes>>, Self::Error>> + Send
    {
        async move {
            let uncles = if block_id.is_pending() {
                // Pending block can be fetched directly without need for caching
                LoadBlock::provider(self)
                    .pending_block()
                    .map_err(Self::Error::from_eth_err)?
                    .map(|block| block.body.ommers)
            } else {
                LoadBlock::provider(self)
                    .ommers_by_id(block_id)
                    .map_err(Self::Error::from_eth_err)?
            }
            .unwrap_or_default();

            Ok(uncles.into_iter().nth(index.into()).map(uncle_block_from_header))
        }
    }
}

/// Loads a block from database.
///
/// Behaviour shared by several `eth_` RPC methods, not exclusive to `eth_` blocks RPC methods.
pub trait LoadBlock: LoadPendingBlock + SpawnBlocking {
    // Returns a handle for reading data from disk.
    ///
    /// Data access in default (L1) trait method implementations.
    fn provider(&self) -> impl BlockReaderIdExt;

    /// Returns a handle for reading data from memory.
    ///
    /// Data access in default (L1) trait method implementations.
    fn cache(&self) -> &EthStateCache;

    /// Returns the block object for the given block id.
    fn block_with_senders(
        &self,
        block_id: BlockId,
    ) -> impl Future<Output = Result<Option<Arc<SealedBlockWithSenders>>, Self::Error>> + Send {
        async move {
            if block_id.is_pending() {
                // Pending block can be fetched directly without need for caching
                let maybe_pending = LoadPendingBlock::provider(self)
                    .pending_block_with_senders()
                    .map_err(Self::Error::from_eth_err)?;
                return if maybe_pending.is_some() {
                    Ok(maybe_pending.map(Arc::new))
                } else {
                    // If no pending block from provider, try to get local pending block
                    return match self.local_pending_block().await? {
                        Some((block, _)) => Ok(Some(Arc::new(block))),
                        None => Ok(None),
                    };
                };
            }

            // We retrieve the block hash for the given block id via the provider
            let block_hash = match LoadPendingBlock::provider(self)
                .block_hash_for_id(block_id)
                .map_err(Self::Error::from_eth_err)?
            {
                Some(block_hash) => block_hash,
                // If the block hash is not found, we return `None` directly
                None => return Ok(None),
            };

            // Initialize the maximum number of retries for handling reorg cases.
            //
            // A reorg may cause the latest block to be temporarily absent from the cache.
            //
            // By retrying once, we give the system a chance to update the cache with
            // the new latest block information after a reorganization.
            let max_retries = 1;

            // Attempt to fetch the block from cache
            for _ in 0..=max_retries {
                match self.cache().get_sealed_block_with_senders(block_hash).await {
                    // If a block is found in the cache, return it
                    Ok(Some(block)) => return Ok(Some(block)),
                    // If no block is found and the `block_id` refers to the latest block,
                    // we retry the fetch, as this may indicate a reorg scenario
                    Ok(None) if block_id.is_latest() => continue,
                    // If no block is found and the `block_id` is not the latest, return `None`
                    Ok(None) => return Ok(None),
                    // If an error occurs while fetching from the cache, return it as an error
                    Err(err) => return Err(Self::Error::from_eth_err(err)),
                }
            }

            // Return `None` if all retries have been exhausted without finding the block
            Ok(None)
        }
    }
}
