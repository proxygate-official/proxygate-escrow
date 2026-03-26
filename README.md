# ProxyGate Escrow

Solana smart contract for the ProxyGate API marketplace. Handles USDC deposits, withdrawals, batch settlements to sellers, and timeout-based buyer reclaim.

## Program

| | |
|---|---|
| **Program ID** | `7fe3uMMqrJjqmTy5rB4CVn1pvNxH6z5Snh8ULa3cQXmo` |
| **Network** | Mainnet-beta |
| **Framework** | Anchor 0.32.1 |
| **Token** | USDC (SPL Token-2022 compatible) |

## Architecture

```
Buyer deposits USDC into a per-buyer PDA vault.
The platform settles usage by transferring from vault to sellers + fee.
Buyers can withdraw with platform co-sign, or reclaim after timeout.

         deposit
Buyer ──────────► BuyerVault (PDA)
                      │
         settle       ├──► Seller 1
Platform ────────►    ├──► Seller 2
                      └──► Platform Fee

         withdraw (co-signed)
Buyer + Platform ──► Buyer wallet

         timeout_reclaim (permissionless)
Anyone ──────────► Buyer wallet (after timeout)
```

## Instructions

| Instruction | Signer(s) | Description |
|---|---|---|
| `initialize_config` | Admin | One-time setup: authority, fee destination, USDC mint, timeout |
| `update_config` | Admin | Update authority, fees, pause state |
| `deposit` | Buyer | Deposit USDC into buyer's vault PDA |
| `withdraw` | Buyer + Platform | Withdraw USDC back to buyer (platform co-sign required) |
| `settle` | Platform | Batch payout to sellers + platform fee with sequence replay protection |
| `timeout_reclaim` | Anyone | Permissionless reclaim of all vault funds after timeout (bypasses pause) |

## Security

- **Withdraw co-sign**: Platform authority must co-sign all withdrawals, preventing frontrun attacks
- **Sequence replay protection**: Each settlement increments a monotonic sequence number
- **Checked arithmetic**: All math uses checked operations, no overflow possible
- **Pause mechanism**: Admin can pause deposits, withdrawals, and settlements
- **Timeout reclaim**: Buyers can always recover funds after timeout, even when paused
- **Seller mint validation**: Settlement validates seller token accounts hold the correct USDC mint
- **PDA isolation**: Each buyer has a dedicated vault PDA, no shared state

## Accounts

| Account | Type | Seeds |
|---|---|---|
| Config | PDA | `["config"]` |
| BuyerVault | PDA | `["vault", buyer_pubkey]` |
| VaultTokenAccount | PDA | `["vault_token", buyer_pubkey]` |

## Build

```bash
anchor build --verifiable
```

## Test

```bash
cargo test -p escrow
```

## Verify

```bash
solana-verify verify-from-repo \
  --program-id 7fe3uMMqrJjqmTy5rB4CVn1pvNxH6z5Snh8ULa3cQXmo \
  https://github.com/proxygate-official/proxygate-escrow \
  -u mainnet-beta
```

## License

BUSL-1.1
