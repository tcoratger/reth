//! Error handling for the blockchain tree

use crate::{
    executor::{BlockExecutionError, BlockValidationError},
    provider::ProviderError,
    RethError,
};
use reth_consensus::ConsensusError;
use reth_primitives::{BlockHash, BlockNumber, SealedBlock};

/// Various error cases that can occur when a block violates tree assumptions.
#[derive(Debug, Clone, Copy, thiserror::Error, Eq, PartialEq)]
pub enum BlockchainTreeError {
    /// Thrown if the block number is lower than the last finalized block number.
    #[error("block number is lower than the last finalized block number #{last_finalized}")]
    PendingBlockIsFinalized {
        /// The block number of the last finalized block.
        last_finalized: BlockNumber,
    },
    /// Thrown if no side chain could be found for the block.
    #[error("chainId can't be found in BlockchainTree with internal index {chain_id}")]
    BlockSideChainIdConsistency {
        /// The internal identifier for the side chain.
        chain_id: u64,
    },
    /// Thrown if a canonical chain header cannot be found.
    #[error("canonical chain header {block_hash} can't be found")]
    CanonicalChain {
        /// The block hash of the missing canonical chain header.
        block_hash: BlockHash,
    },
    /// Thrown if a block number cannot be found in the blockchain tree chain.
    #[error("block number #{block_number} not found in blockchain tree chain")]
    BlockNumberNotFoundInChain {
        /// The block number that could not be found.
        block_number: BlockNumber,
    },
    /// Thrown if a block hash cannot be found in the blockchain tree chain.
    #[error("block hash {block_hash} not found in blockchain tree chain")]
    BlockHashNotFoundInChain {
        /// The block hash that could not be found.
        block_hash: BlockHash,
    },
    /// Thrown if the block failed to buffer
    #[error("block with hash {block_hash} failed to buffer")]
    BlockBufferingFailed {
        /// The block hash of the block that failed to buffer.
        block_hash: BlockHash,
    },
    /// Thrown when trying to access genesis parent.
    #[error("genesis block has no parent")]
    GenesisBlockHasNoParent,
}

/// Canonical Errors
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum CanonicalError {
    /// Error originating from validation operations.
    #[error(transparent)]
    Validation(#[from] BlockValidationError),
    /// Error originating from blockchain tree operations.
    #[error(transparent)]
    BlockchainTree(#[from] BlockchainTreeError),
    /// Error originating from a provider operation.
    #[error(transparent)]
    Provider(#[from] ProviderError),
    /// Error indicating a transaction reverted during execution.
    #[error("transaction error on revert: {0}")]
    CanonicalRevert(String),
    /// Error indicating a transaction failed to commit during execution.
    #[error("transaction error on commit: {0}")]
    CanonicalCommit(String),
    /// Error indicating that a previous optimistic sync target was re-orged
    #[error("transaction error on revert: {0}")]
    OptimisticTargetRevert(BlockNumber),
}

impl CanonicalError {
    /// Returns `true` if the error is fatal.
    pub fn is_fatal(&self) -> bool {
        matches!(self, Self::CanonicalCommit(_) | Self::CanonicalRevert(_))
    }

    /// Returns `true` if the underlying error matches
    /// [BlockchainTreeError::BlockHashNotFoundInChain].
    pub fn is_block_hash_not_found(&self) -> bool {
        matches!(self, Self::BlockchainTree(BlockchainTreeError::BlockHashNotFoundInChain { .. }))
    }

    /// Returns `Some(BlockNumber)` if the underlying error matches
    /// [CanonicalError::OptimisticTargetRevert].
    pub fn optimistic_revert_block_number(&self) -> Option<BlockNumber> {
        match self {
            Self::OptimisticTargetRevert(block_number) => Some(*block_number),
            _ => None,
        }
    }
}

/// Error thrown when inserting a block failed because the block is considered invalid.
#[derive(thiserror::Error)]
#[error(transparent)]
pub struct InsertBlockError {
    inner: Box<InsertBlockErrorData>,
}

// === impl InsertBlockError ===

impl InsertBlockError {
    /// Create a new InsertInvalidBlockError
    pub fn new(block: SealedBlock, kind: InsertBlockErrorKind) -> Self {
        Self { inner: InsertBlockErrorData::boxed(block, kind) }
    }

    /// Create a new InsertInvalidBlockError from a tree error
    pub fn tree_error(error: BlockchainTreeError, block: SealedBlock) -> Self {
        Self::new(block, InsertBlockErrorKind::Tree(error))
    }

    /// Create a new InsertInvalidBlockError from a consensus error
    pub fn consensus_error(error: ConsensusError, block: SealedBlock) -> Self {
        Self::new(block, InsertBlockErrorKind::Consensus(error))
    }

    /// Create a new InsertInvalidBlockError from a consensus error
    pub fn sender_recovery_error(block: SealedBlock) -> Self {
        Self::new(block, InsertBlockErrorKind::SenderRecovery)
    }

    /// Create a new InsertInvalidBlockError from an execution error
    pub fn execution_error(error: BlockExecutionError, block: SealedBlock) -> Self {
        Self::new(block, InsertBlockErrorKind::Execution(error))
    }

    /// Create a new InsertBlockError from a RethError and block.
    pub fn from_reth_error(error: RethError, block: SealedBlock) -> Self {
        Self::new(block, error.into())
    }

    /// Consumes the error and returns the block that resulted in the error
    #[inline]
    pub fn into_block(self) -> SealedBlock {
        self.inner.block
    }

    /// Returns the error kind
    #[inline]
    pub fn kind(&self) -> &InsertBlockErrorKind {
        &self.inner.kind
    }

    /// Returns the block that resulted in the error
    #[inline]
    pub fn block(&self) -> &SealedBlock {
        &self.inner.block
    }

    /// Consumes the type and returns the block and error kind.
    #[inline]
    pub fn split(self) -> (SealedBlock, InsertBlockErrorKind) {
        let inner = *self.inner;
        (inner.block, inner.kind)
    }
}

impl std::fmt::Debug for InsertBlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.inner, f)
    }
}

struct InsertBlockErrorData {
    block: SealedBlock,
    kind: InsertBlockErrorKind,
}

impl std::fmt::Display for InsertBlockErrorData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Failed to insert block (hash={}, number={}, parent_hash={}): {}",
            self.block.hash(),
            self.block.number,
            self.block.parent_hash,
            self.kind
        )
    }
}

impl std::fmt::Debug for InsertBlockErrorData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InsertBlockError")
            .field("error", &self.kind)
            .field("hash", &self.block.hash())
            .field("number", &self.block.number)
            .field("parent_hash", &self.block.parent_hash)
            .field("num_txs", &self.block.body.len())
            .finish_non_exhaustive()
    }
}

impl std::error::Error for InsertBlockErrorData {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
    }
}

impl InsertBlockErrorData {
    fn new(block: SealedBlock, kind: InsertBlockErrorKind) -> Self {
        Self { block, kind }
    }

    fn boxed(block: SealedBlock, kind: InsertBlockErrorKind) -> Box<Self> {
        Box::new(Self::new(block, kind))
    }
}

/// All error variants possible when inserting a block
#[derive(Debug, thiserror::Error)]
pub enum InsertBlockErrorKind {
    /// Failed to recover senders for the block
    #[error("failed to recover senders for block")]
    SenderRecovery,
    /// Block violated consensus rules.
    #[error(transparent)]
    Consensus(#[from] ConsensusError),
    /// Block execution failed.
    #[error(transparent)]
    Execution(#[from] BlockExecutionError),
    /// Block violated tree invariants.
    #[error(transparent)]
    Tree(#[from] BlockchainTreeError),
    /// Provider error.
    #[error(transparent)]
    Provider(#[from] ProviderError),
    /// An internal error occurred, like interacting with the database.
    #[error(transparent)]
    Internal(#[from] Box<dyn std::error::Error + Send + Sync>),
    /// Canonical error.
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    /// BlockchainTree error.
    #[error(transparent)]
    BlockchainTree(BlockchainTreeError),
}

impl InsertBlockErrorKind {
    /// Returns true if the error is a tree error
    pub fn is_tree_error(&self) -> bool {
        matches!(self, Self::Tree(_))
    }

    /// Returns true if the error is a consensus error
    pub fn is_consensus_error(&self) -> bool {
        matches!(self, Self::Consensus(_))
    }

    /// Returns true if this error is a state root error
    pub fn is_state_root_error(&self) -> bool {
        // we need to get the state root errors inside of the different variant branches
        match self {
            Self::Execution(err) => {
                matches!(
                    err,
                    BlockExecutionError::Validation(BlockValidationError::StateRoot { .. })
                )
            }
            Self::Canonical(err) => {
                matches!(
                    err,
                    CanonicalError::Validation(BlockValidationError::StateRoot { .. }) |
                        CanonicalError::Provider(
                            ProviderError::StateRootMismatch(_) |
                                ProviderError::UnwindStateRootMismatch(_)
                        )
                )
            }
            Self::Provider(err) => {
                matches!(
                    err,
                    ProviderError::StateRootMismatch(_) | ProviderError::UnwindStateRootMismatch(_)
                )
            }
            _ => false,
        }
    }

    /// Returns true if the error is caused by an invalid block
    ///
    /// This is intended to be used to determine if the block should be marked as invalid.
    pub fn is_invalid_block(&self) -> bool {
        match self {
            Self::SenderRecovery | Self::Consensus(_) => true,
            // other execution errors that are considered internal errors
            Self::Execution(err) => {
                match err {
                    BlockExecutionError::Validation(_) => {
                        // this is caused by an invalid block
                        true
                    }
                    // these are internal errors, not caused by an invalid block
                    BlockExecutionError::LatestBlock(_) |
                    BlockExecutionError::Pruning(_) |
                    BlockExecutionError::CanonicalRevert { .. } |
                    BlockExecutionError::CanonicalCommit { .. } |
                    BlockExecutionError::AppendChainDoesntConnect { .. } |
                    BlockExecutionError::UnavailableForTest => false,
                    BlockExecutionError::Other(_) => false,
                }
            }
            Self::Tree(err) => {
                match err {
                    BlockchainTreeError::PendingBlockIsFinalized { .. } => {
                        // the block's number is lower than the finalized block's number
                        true
                    }
                    BlockchainTreeError::BlockSideChainIdConsistency { .. } |
                    BlockchainTreeError::CanonicalChain { .. } |
                    BlockchainTreeError::BlockNumberNotFoundInChain { .. } |
                    BlockchainTreeError::BlockHashNotFoundInChain { .. } |
                    BlockchainTreeError::BlockBufferingFailed { .. } |
                    BlockchainTreeError::GenesisBlockHasNoParent => false,
                }
            }
            Self::Provider(_) | Self::Internal(_) => {
                // any other error, such as database errors, are considered internal errors
                false
            }
            Self::Canonical(err) => match err {
                CanonicalError::BlockchainTree(_) |
                CanonicalError::CanonicalCommit(_) |
                CanonicalError::CanonicalRevert(_) |
                CanonicalError::OptimisticTargetRevert(_) => false,
                CanonicalError::Validation(_) => true,
                CanonicalError::Provider(_) => false,
            },
            Self::BlockchainTree(_) => false,
        }
    }

    /// Returns true if this is a block pre merge error.
    pub fn is_block_pre_merge(&self) -> bool {
        matches!(
            self,
            Self::Execution(BlockExecutionError::Validation(
                BlockValidationError::BlockPreMerge { .. }
            ))
        )
    }

    /// Returns true if the error is an execution error
    pub fn is_execution_error(&self) -> bool {
        matches!(self, Self::Execution(_))
    }

    /// Returns true if the error is an internal error
    pub fn is_internal(&self) -> bool {
        matches!(self, Self::Internal(_))
    }

    /// Returns the error if it is a tree error
    pub fn as_tree_error(&self) -> Option<BlockchainTreeError> {
        match self {
            Self::Tree(err) => Some(*err),
            _ => None,
        }
    }

    /// Returns the error if it is a consensus error
    pub fn as_consensus_error(&self) -> Option<&ConsensusError> {
        match self {
            Self::Consensus(err) => Some(err),
            _ => None,
        }
    }

    /// Returns the error if it is an execution error
    pub fn as_execution_error(&self) -> Option<&BlockExecutionError> {
        match self {
            Self::Execution(err) => Some(err),
            _ => None,
        }
    }
}

// This is a convenience impl to convert from crate::Error to InsertBlockErrorKind
impl From<RethError> for InsertBlockErrorKind {
    fn from(err: RethError) -> Self {
        match err {
            RethError::Execution(err) => Self::Execution(err),
            RethError::Consensus(err) => Self::Consensus(err),
            RethError::Database(err) => Self::Internal(Box::new(err)),
            RethError::Provider(err) => Self::Internal(Box::new(err)),
            RethError::Network(err) => Self::Internal(Box::new(err)),
            RethError::Custom(err) => Self::Internal(err.into()),
            RethError::Canonical(err) => Self::Canonical(err),
        }
    }
}
