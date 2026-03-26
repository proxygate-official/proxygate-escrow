use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct BuyerVault {
    /// The buyer's wallet public key
    pub buyer: Pubkey, // 32 bytes
    /// Monotonically increasing sequence number (starts at 0)
    pub sequence: u64, // 8 bytes
    /// Unix timestamp of last settlement (for timeout_reclaim)
    pub last_settled_at: i64, // 8 bytes
    /// PDA bump seed
    pub bump: u8, // 1 byte
    /// Token account PDA bump seed
    pub token_bump: u8, // 1 byte
}
// Total with discriminator: 8 + 32 + 8 + 8 + 1 + 1 = 58 bytes

#[account]
#[derive(InitSpace)]
pub struct Config {
    /// Admin who can update config (later → Squads multisig)
    pub admin: Pubkey, // 32
    /// Authority that signs settle/withdraw instructions
    pub authority: Pubkey, // 32
    /// Receives platform fees (separate from authority for Squads treasury)
    pub fee_destination: Pubkey, // 32
    /// USDC mint address (immutable after init)
    pub usdc_mint: Pubkey, // 32
    /// Platform fee in basis points (informational)
    pub fee_bps: u16, // 2
    /// Timeout for timeout_reclaim in seconds
    pub timeout_seconds: i64, // 8
    /// Emergency pause flag
    pub paused: bool, // 1
    /// PDA bump seed
    pub bump: u8, // 1
}
// PDA seed: [b"config"], total with discriminator: 8 + 140 = 148 bytes
