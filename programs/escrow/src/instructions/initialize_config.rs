use anchor_lang::prelude::*;

use crate::state::Config;

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        init,
        payer = admin,
        space = 8 + Config::INIT_SPACE,
        seeds = [b"config"],
        bump,
    )]
    pub config: Account<'info, Config>,

    pub system_program: Program<'info, System>,
}

pub fn initialize_config(
    ctx: Context<InitializeConfig>,
    authority: Pubkey,
    fee_destination: Pubkey,
    usdc_mint: Pubkey,
    fee_bps: u16,
    timeout_seconds: i64,
) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.admin = ctx.accounts.admin.key();
    config.authority = authority;
    config.fee_destination = fee_destination;
    config.usdc_mint = usdc_mint;
    config.fee_bps = fee_bps;
    config.timeout_seconds = timeout_seconds;
    config.paused = false;
    config.bump = ctx.bumps.config;
    Ok(())
}
