use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::error::EscrowError;
use crate::state::{BuyerVault, Config};

#[derive(Accounts)]
pub struct Settle<'info> {
    #[account(
        seeds = [b"config"],
        bump = config.bump,
        constraint = !config.paused @ EscrowError::Paused,
    )]
    pub config: Account<'info, Config>,

    /// Platform authority — validated against config.authority
    #[account(address = config.authority)]
    pub platform: Signer<'info>,

    /// The buyer vault PDA being settled against
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

    /// Fee destination's USDC token account for fee collection
    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = config.fee_destination,
        associated_token::token_program = token_program,
    )]
    pub platform_token_account: InterfaceAccount<'info, TokenAccount>,

    /// USDC mint — validated against config
    #[account(address = config.usdc_mint)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

/// Batch settlement: debits buyer vault and distributes to multiple sellers + platform fee.
///
/// remaining_accounts: seller token accounts (one per seller, paired with seller_amounts)
/// expected_sequence: must equal vault.sequence + 1 (replay protection)
/// seller_amounts: USDC amounts (in smallest unit) for each seller
/// platform_fee: USDC amount for platform fee collection
pub fn settle<'a, 'b, 'c, 'info>(
    ctx: Context<'a, 'b, 'c, 'info, Settle<'info>>,
    expected_sequence: u64,
    seller_amounts: Vec<u64>,
    platform_fee: u64,
) -> Result<()> {
    let vault = &mut ctx.accounts.vault;

    // --- Sequence validation (replay protection) ---
    let next_sequence = vault
        .sequence
        .checked_add(1)
        .ok_or(error!(EscrowError::ArithmeticOverflow))?;
    require!(
        expected_sequence == next_sequence,
        EscrowError::InvalidSequence
    );

    // --- Seller count validation ---
    require!(
        ctx.remaining_accounts.len() == seller_amounts.len(),
        EscrowError::SellerCountMismatch
    );
    require!(!seller_amounts.is_empty(), EscrowError::ZeroAmount);

    // --- Total debit validation (checked arithmetic) ---
    let seller_total = seller_amounts
        .iter()
        .try_fold(0u64, |acc, &amt| acc.checked_add(amt))
        .ok_or(error!(EscrowError::ArithmeticOverflow))?;

    let total_debit = seller_total
        .checked_add(platform_fee)
        .ok_or(error!(EscrowError::ArithmeticOverflow))?;

    require!(
        ctx.accounts.vault_token_account.amount >= total_debit,
        EscrowError::InsufficientBalance
    );

    // --- PDA signer seeds ---
    let buyer_key = vault.buyer;
    let bump = vault.bump;
    let signer_seeds: &[&[&[u8]]] = &[&[b"vault", buyer_key.as_ref(), &[bump]]];

    // --- Transfer to each seller ---
    let usdc_mint_key = ctx.accounts.usdc_mint.key();
    for (i, seller_token_account) in ctx.remaining_accounts.iter().enumerate() {
        let amount = seller_amounts[i];
        if amount == 0 {
            continue;
        }

        // Validate seller token account has correct USDC mint
        let seller_data = seller_token_account.try_borrow_data()?;
        // SPL token account mint is at bytes 0..32
        require!(seller_data.len() >= 72, EscrowError::InvalidSellerMint);
        let mint_bytes: [u8; 32] = seller_data[0..32].try_into().unwrap();
        require!(
            Pubkey::from(mint_bytes) == usdc_mint_key,
            EscrowError::InvalidSellerMint
        );
        drop(seller_data);

        let cpi_accounts = TransferChecked {
            from: ctx.accounts.vault_token_account.to_account_info(),
            mint: ctx.accounts.usdc_mint.to_account_info(),
            to: seller_token_account.to_account_info(),
            authority: ctx.accounts.vault.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        token_interface::transfer_checked(cpi_ctx, amount, 6)?;
    }

    // --- Transfer platform fee ---
    if platform_fee > 0 {
        let cpi_accounts = TransferChecked {
            from: ctx.accounts.vault_token_account.to_account_info(),
            mint: ctx.accounts.usdc_mint.to_account_info(),
            to: ctx.accounts.platform_token_account.to_account_info(),
            authority: ctx.accounts.vault.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        token_interface::transfer_checked(cpi_ctx, platform_fee, 6)?;
    }

    // --- Update vault state ---
    let vault = &mut ctx.accounts.vault;
    vault.sequence = expected_sequence;
    vault.last_settled_at = Clock::get()?.unix_timestamp;

    Ok(())
}
