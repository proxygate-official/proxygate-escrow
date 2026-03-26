use anchor_lang::prelude::*;

#[error_code]
pub enum EscrowError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Insufficient vault balance")]
    InsufficientBalance,
    #[msg("Invalid sequence number -- expected vault.sequence + 1")]
    InvalidSequence,
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
    #[msg("Number of seller accounts does not match seller_amounts length")]
    SellerCountMismatch,
    #[msg("Timeout period has not elapsed")]
    TimeoutNotReached,
    #[msg("Vault has no funds to reclaim")]
    VaultEmpty,
    #[msg("Program is paused")]
    Paused,
    #[msg("Unauthorized: signer is not the config admin")]
    UnauthorizedAdmin,
    #[msg("Seller token account has wrong mint")]
    InvalidSellerMint,
}
