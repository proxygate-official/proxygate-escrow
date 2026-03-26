use anchor_lang::prelude::*;

mod error;
mod instructions;
mod state;

use instructions::*;

#[cfg(not(feature = "no-entrypoint"))]
use solana_security_txt::security_txt;

#[cfg(not(feature = "no-entrypoint"))]
security_txt! {
    name: "ProxyGate Escrow",
    project_url: "https://proxygate.ai",
    contacts: "email:info@proxygate.ai",
    policy: "https://proxygate.ai",
    preferred_languages: "en",
    source_code: "https://github.com/proxygate-official/proxygate-escrow"
}

declare_id!("7fe3uMMqrJjqmTy5rB4CVn1pvNxH6z5Snh8ULa3cQXmo");

#[program]
pub mod escrow {
    use super::*;

    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        authority: Pubkey,
        fee_destination: Pubkey,
        usdc_mint: Pubkey,
        fee_bps: u16,
        timeout_seconds: i64,
    ) -> Result<()> {
        instructions::initialize_config::initialize_config(
            ctx,
            authority,
            fee_destination,
            usdc_mint,
            fee_bps,
            timeout_seconds,
        )
    }

    pub fn update_config(
        ctx: Context<UpdateConfig>,
        new_admin: Option<Pubkey>,
        new_authority: Option<Pubkey>,
        new_fee_destination: Option<Pubkey>,
        new_fee_bps: Option<u16>,
        new_timeout_seconds: Option<i64>,
        new_paused: Option<bool>,
    ) -> Result<()> {
        instructions::update_config::update_config(
            ctx,
            new_admin,
            new_authority,
            new_fee_destination,
            new_fee_bps,
            new_timeout_seconds,
            new_paused,
        )
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        instructions::deposit::deposit(ctx, amount)
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        instructions::withdraw::withdraw(ctx, amount)
    }

    pub fn settle<'a, 'b, 'c, 'info>(
        ctx: Context<'a, 'b, 'c, 'info, Settle<'info>>,
        expected_sequence: u64,
        seller_amounts: Vec<u64>,
        platform_fee: u64,
    ) -> Result<()> {
        instructions::settle::settle(ctx, expected_sequence, seller_amounts, platform_fee)
    }

    pub fn timeout_reclaim(ctx: Context<TimeoutReclaim>) -> Result<()> {
        instructions::timeout_reclaim::timeout_reclaim(ctx)
    }
}
