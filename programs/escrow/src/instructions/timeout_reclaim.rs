use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::error::EscrowError;
use crate::state::{BuyerVault, Config};

#[derive(Accounts)]
pub struct TimeoutReclaim<'info> {
    /// Config PDA — NO pause check (buyer protection always works)
    #[account(
        seeds = [b"config"],
        bump = config.bump,
    )]
    pub config: Account<'info, Config>,

    /// The buyer whose vault is being reclaimed.
    /// NOT a signer -- this is permissionless (anyone can trigger reclaim).
    /// CHECK: Validated by constraint below (must match vault.buyer).
    #[account(constraint = buyer.key() == vault.buyer)]
    pub buyer: UncheckedAccount<'info>,

    /// The person triggering the reclaim (can be anyone, permissionless)
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The buyer vault PDA
    #[account(
        mut,
        seeds = [b"vault", vault.buyer.as_ref()],
        bump = vault.bump,
    )]
    pub vault: Account<'info, BuyerVault>,

    /// The vault's USDC token account (PDA-owned)
    #[account(
        mut,
        seeds = [b"vault_token", vault.buyer.as_ref()],
        bump = vault.token_bump,
        token::mint = usdc_mint,
        token::authority = vault,
        token::token_program = token_program,
    )]
    pub vault_token_account: InterfaceAccount<'info, TokenAccount>,

    /// The buyer's ATA where reclaimed funds go
    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = buyer,
        associated_token::token_program = token_program,
    )]
    pub buyer_token_account: InterfaceAccount<'info, TokenAccount>,

    /// USDC mint — validated against config
    #[account(address = config.usdc_mint)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
}

/// Permissionless timeout reclaim: if no settlement for timeout_seconds,
/// anyone can transfer ALL vault funds back to the buyer.
pub fn timeout_reclaim(ctx: Context<TimeoutReclaim>) -> Result<()> {
    let clock = Clock::get()?;
    let timeout_seconds = ctx.accounts.config.timeout_seconds;

    // --- Timeout validation ---
    let elapsed = clock
        .unix_timestamp
        .checked_sub(ctx.accounts.vault.last_settled_at)
        .ok_or(error!(EscrowError::ArithmeticOverflow))?;

    require!(elapsed >= timeout_seconds, EscrowError::TimeoutNotReached);

    // --- Balance check ---
    let amount = ctx.accounts.vault_token_account.amount;
    require!(amount > 0, EscrowError::VaultEmpty);

    // --- Transfer ALL funds to buyer ---
    let buyer_key = ctx.accounts.vault.buyer;
    let bump = ctx.accounts.vault.bump;
    let signer_seeds: &[&[&[u8]]] = &[&[b"vault", buyer_key.as_ref(), &[bump]]];

    let cpi_accounts = TransferChecked {
        from: ctx.accounts.vault_token_account.to_account_info(),
        mint: ctx.accounts.usdc_mint.to_account_info(),
        to: ctx.accounts.buyer_token_account.to_account_info(),
        authority: ctx.accounts.vault.to_account_info(),
    };
    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        cpi_accounts,
        signer_seeds,
    );
    token_interface::transfer_checked(cpi_ctx, amount, 6)?;

    // --- Update vault state ---
    let vault = &mut ctx.accounts.vault;
    vault.last_settled_at = clock.unix_timestamp;

    Ok(())
}
