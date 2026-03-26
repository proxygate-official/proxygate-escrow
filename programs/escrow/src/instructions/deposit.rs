use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::error::EscrowError;
use crate::state::{BuyerVault, Config};

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(
        seeds = [b"config"],
        bump = config.bump,
        constraint = !config.paused @ EscrowError::Paused,
    )]
    pub config: Account<'info, Config>,

    #[account(mut)]
    pub buyer: Signer<'info>,

    #[account(
        init_if_needed,
        payer = buyer,
        space = 8 + BuyerVault::INIT_SPACE,
        seeds = [b"vault", buyer.key().as_ref()],
        bump,
    )]
    pub vault: Account<'info, BuyerVault>,

    #[account(
        init_if_needed,
        payer = buyer,
        token::mint = usdc_mint,
        token::authority = vault,
        token::token_program = token_program,
        seeds = [b"vault_token", buyer.key().as_ref()],
        bump,
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
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
    require!(amount > 0, EscrowError::ZeroAmount);

    let vault = &mut ctx.accounts.vault;

    // First-time initialization: buyer field is default (all zeros)
    if vault.buyer == Pubkey::default() {
        vault.buyer = ctx.accounts.buyer.key();
        vault.sequence = 0;
        vault.last_settled_at = Clock::get()?.unix_timestamp;
        vault.bump = ctx.bumps.vault;
        vault.token_bump = ctx.bumps.vault_token_account;
    }

    // Transfer USDC from buyer to vault token account
    let cpi_accounts = TransferChecked {
        from: ctx.accounts.buyer_token_account.to_account_info(),
        mint: ctx.accounts.usdc_mint.to_account_info(),
        to: ctx.accounts.vault_token_account.to_account_info(),
        authority: ctx.accounts.buyer.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
    token_interface::transfer_checked(cpi_ctx, amount, 6)?;

    Ok(())
}
