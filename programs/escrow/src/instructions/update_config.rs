use anchor_lang::prelude::*;

use crate::error::EscrowError;
use crate::state::Config;

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(constraint = admin.key() == config.admin @ EscrowError::UnauthorizedAdmin)]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"config"],
        bump = config.bump,
    )]
    pub config: Account<'info, Config>,
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
    let config = &mut ctx.accounts.config;

    if let Some(admin) = new_admin {
        config.admin = admin;
    }
    if let Some(authority) = new_authority {
        config.authority = authority;
    }
    if let Some(fee_destination) = new_fee_destination {
        config.fee_destination = fee_destination;
    }
    if let Some(fee_bps) = new_fee_bps {
        config.fee_bps = fee_bps;
    }
    if let Some(timeout_seconds) = new_timeout_seconds {
        config.timeout_seconds = timeout_seconds;
    }
    if let Some(paused) = new_paused {
        config.paused = paused;
    }

    Ok(())
}
