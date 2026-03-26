use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::error::EscrowError;
use crate::state::{BuyerVault, Config};

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(
        seeds = [b"config"],
        bump = config.bump,
        constraint = !config.paused @ EscrowError::Paused,
    )]
    pub config: Account<'info, Config>,

    #[account(mut)]
    pub buyer: Signer<'info>,

    /// Platform authority — must co-sign all withdrawals.
    #[account(address = config.authority)]
    pub platform: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", buyer.key().as_ref()],
        bump = vault.bump,
        has_one = buyer,
    )]
    pub vault: Account<'info, BuyerVault>,

    #[account(
        mut,
        seeds = [b"vault_token", buyer.key().as_ref()],
        bump = vault.token_bump,
        token::mint = usdc_mint,
        token::authority = vault,
        token::token_program = token_program,
    )]
    pub vault_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = buyer,
        associated_token::token_program = token_program,
    )]
    pub buyer_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(address = config.usdc_mint)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
    require!(amount > 0, EscrowError::ZeroAmount);
    require!(
        ctx.accounts.vault_token_account.amount >= amount,
        EscrowError::InsufficientBalance
    );

    // Build PDA signer seeds for vault authority
    let buyer_key = ctx.accounts.vault.buyer;
    let bump = ctx.accounts.vault.bump;
    let signer_seeds: &[&[&[u8]]] = &[&[b"vault", buyer_key.as_ref(), &[bump]]];

    // Transfer USDC from vault token account back to buyer
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

    Ok(())
}
