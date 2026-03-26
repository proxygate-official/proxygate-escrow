use litesvm::LiteSVM;
use sha2::Digest;
use solana_account::Account;
use solana_address::Address;
use solana_clock::Clock;
use solana_instruction::{account_meta::AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_program_option::COption;
use solana_program_pack::Pack;
use solana_signer::Signer;
use solana_transaction::Transaction;
use spl_associated_token_account_interface::address::get_associated_token_address;
use spl_token_interface::{
    state::{Account as SplTokenAccount, AccountState, Mint as SplMint},
    ID as TOKEN_PROGRAM_ID,
};

// The program's declared ID (updated for config PDA build)
const PROGRAM_ID_STR: &str = "7fe3uMMqrJjqmTy5rB4CVn1pvNxH6z5Snh8ULa3cQXmo";

// USDC_MINT — now configured via Config PDA, but tests still use a known address
const USDC_MINT_STR: &str = "FED9q6ZxwjiwHtQ3Rc3CJgpFqiME9txNgNbEdLLs3q2H";

// TIMEOUT_SECONDS — configured via Config PDA
const TIMEOUT_SECONDS: i64 = 7 * 24 * 60 * 60;

// Platform fee BPS — configured via Config PDA
const PLATFORM_FEE_BPS: u16 = 500;

// Deterministic test-only keypair (NOT a production key).
// Derived from a fixed seed so tests are reproducible.
fn test_platform_keypair() -> Keypair {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(b"proxygate-escrow-test-platform-keypair");
    let seed: [u8; 32] = hash.into();
    Keypair::new_from_array(seed)
}

// Associated Token Program ID
const ASSOCIATED_TOKEN_PROGRAM_ID: Address =
    solana_address::address!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

// System Program ID
const SYSTEM_PROGRAM_ID: Address =
    solana_address::address!("11111111111111111111111111111111");

/// Compute Anchor instruction discriminator: first 8 bytes of sha256("global:{method_name}")
fn anchor_discriminator(method_name: &str) -> [u8; 8] {
    let hash = sha2::Sha256::digest(format!("global:{}", method_name));
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
}

#[allow(dead_code)]
struct TestEnv {
    svm: LiteSVM,
    program_id: Address,
    platform_keypair: Keypair,
    usdc_mint: Address,
    mint_authority: Keypair,
    config_pda: Address,
    fee_destination: Keypair,
}

impl TestEnv {
    fn new() -> Self {
        let mut svm = LiteSVM::new();

        // Set clock to a realistic timestamp (LiteSVM starts at 0)
        let mut clock: Clock = svm.get_sysvar();
        clock.unix_timestamp = 1_700_000_000; // Nov 2023 epoch
        svm.set_sysvar(&clock);

        let program_id: Address = PROGRAM_ID_STR.parse().unwrap();
        let usdc_mint: Address = USDC_MINT_STR.parse().unwrap();

        // Load the escrow program (path relative to workspace root)
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let workspace_root = std::path::Path::new(manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let program_path = workspace_root.join("target/deploy/escrow.so");
        svm.add_program_from_file(program_id, &program_path)
            .expect("Failed to load escrow program");

        // Create a mint authority keypair
        let mint_authority = Keypair::new();
        svm.airdrop(&mint_authority.pubkey(), 10_000_000_000)
            .unwrap();

        // Inject a valid SPL Token Mint at the USDC_MINT address using set_account
        let mut mint_data = [0u8; SplMint::LEN];
        let mint_state = SplMint {
            mint_authority: COption::Some(mint_authority.pubkey()),
            supply: 0,
            decimals: 6,
            is_initialized: true,
            freeze_authority: COption::None,
        };
        SplMint::pack(mint_state, &mut mint_data).unwrap();

        svm.set_account(
            usdc_mint,
            Account {
                lamports: 1_000_000_000,
                data: mint_data.to_vec(),
                owner: TOKEN_PROGRAM_ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();

        // Load the platform keypair (authority for settle/withdraw)
        let platform_keypair = test_platform_keypair();
        svm.airdrop(&platform_keypair.pubkey(), 10_000_000_000)
            .unwrap();

        // Create a separate fee_destination keypair (receives platform fees)
        let fee_destination = Keypair::new();
        svm.airdrop(&fee_destination.pubkey(), 10_000_000_000)
            .unwrap();

        // Create fee_destination's USDC ATA
        let fee_dest_ata = get_associated_token_address(&fee_destination.pubkey(), &usdc_mint);
        Self::inject_token_account(&mut svm, &fee_dest_ata, &usdc_mint, &fee_destination.pubkey(), 0);

        // Derive Config PDA
        let (config_pda, _) = Address::find_program_address(&[b"config"], &program_id);

        let mut env = TestEnv {
            svm,
            program_id,
            platform_keypair,
            usdc_mint,
            mint_authority,
            config_pda,
            fee_destination,
        };

        // Initialize Config PDA
        env.initialize_config().expect("Failed to initialize config");

        env
    }

    /// Calls initialize_config instruction.
    fn initialize_config(&mut self) -> Result<(), litesvm::types::FailedTransactionMetadata> {
        let disc = anchor_discriminator("initialize_config");
        let mut data = Vec::new();
        data.extend_from_slice(&disc);
        // authority: Pubkey (32)
        data.extend_from_slice(self.platform_keypair.pubkey().as_ref());
        // fee_destination: Pubkey (32)
        data.extend_from_slice(self.fee_destination.pubkey().as_ref());
        // usdc_mint: Pubkey (32)
        data.extend_from_slice(self.usdc_mint.as_ref());
        // fee_bps: u16 (LE)
        data.extend_from_slice(&PLATFORM_FEE_BPS.to_le_bytes());
        // timeout_seconds: i64 (LE)
        data.extend_from_slice(&TIMEOUT_SECONDS.to_le_bytes());

        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.platform_keypair.pubkey(), true), // admin = payer
                AccountMeta::new(self.config_pda, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
            data,
        };

        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.platform_keypair.pubkey()),
            &[&self.platform_keypair],
            self.svm.latest_blockhash(),
        );

        self.svm.send_transaction(tx).map(|_| ())
    }

    /// Injects a valid SPL Token Account at the given address.
    fn inject_token_account(
        svm: &mut LiteSVM,
        address: &Address,
        mint: &Address,
        owner: &Address,
        amount: u64,
    ) {
        let mut data = [0u8; SplTokenAccount::LEN];
        let token_account = SplTokenAccount {
            mint: *mint,
            owner: *owner,
            amount,
            delegate: COption::None,
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        };
        SplTokenAccount::pack(token_account, &mut data).unwrap();

        svm.set_account(
            *address,
            Account {
                lamports: 1_000_000_000,
                data: data.to_vec(),
                owner: TOKEN_PROGRAM_ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
    }

    /// Creates a buyer keypair with SOL and USDC in their ATA.
    fn create_buyer(&mut self, usdc_amount: u64) -> Keypair {
        let buyer = Keypair::new();
        self.svm.airdrop(&buyer.pubkey(), 10_000_000_000).unwrap();

        // Inject buyer's USDC ATA with the specified amount
        let buyer_ata = get_associated_token_address(&buyer.pubkey(), &self.usdc_mint);
        Self::inject_token_account(&mut self.svm, &buyer_ata, &self.usdc_mint, &buyer.pubkey(), usdc_amount);

        buyer
    }

    /// Creates a seller keypair with SOL and a USDC ATA.
    fn create_seller(&mut self) -> Keypair {
        let seller = Keypair::new();
        self.svm.airdrop(&seller.pubkey(), 10_000_000_000).unwrap();

        // Inject seller's USDC ATA with 0 balance
        let seller_ata = get_associated_token_address(&seller.pubkey(), &self.usdc_mint);
        Self::inject_token_account(&mut self.svm, &seller_ata, &self.usdc_mint, &seller.pubkey(), 0);

        seller
    }

    fn get_vault_pda(&self, buyer: &Address) -> (Address, u8) {
        Address::find_program_address(&[b"vault", buyer.as_ref()], &self.program_id)
    }

    fn get_vault_token_pda(&self, buyer: &Address) -> (Address, u8) {
        Address::find_program_address(&[b"vault_token", buyer.as_ref()], &self.program_id)
    }

    /// Builds and sends a deposit instruction.
    fn deposit(
        &mut self,
        buyer: &Keypair,
        amount: u64,
    ) -> Result<(), litesvm::types::FailedTransactionMetadata> {
        let (vault_pda, _) = self.get_vault_pda(&buyer.pubkey());
        let (vault_token_pda, _) = self.get_vault_token_pda(&buyer.pubkey());
        let buyer_ata = get_associated_token_address(&buyer.pubkey(), &self.usdc_mint);

        let disc = anchor_discriminator("deposit");
        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&disc);
        data.extend_from_slice(&amount.to_le_bytes());

        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.config_pda, false), // config PDA
                AccountMeta::new(buyer.pubkey(), true),
                AccountMeta::new(vault_pda, false),
                AccountMeta::new(vault_token_pda, false),
                AccountMeta::new(buyer_ata, false),
                AccountMeta::new_readonly(self.usdc_mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            ],
            data,
        };

        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&buyer.pubkey()),
            &[buyer],
            self.svm.latest_blockhash(),
        );

        self.svm.send_transaction(tx).map(|_| ())
    }

    /// Builds and sends a settle instruction.
    fn settle(
        &mut self,
        buyer_pubkey: &Address,
        expected_sequence: u64,
        seller_pubkeys: &[Address],
        seller_amounts: &[u64],
        platform_fee: u64,
    ) -> Result<(), litesvm::types::FailedTransactionMetadata> {
        let (vault_pda, _) = self.get_vault_pda(buyer_pubkey);
        let (vault_token_pda, _) = self.get_vault_token_pda(buyer_pubkey);
        // Fee goes to fee_destination's ATA (not platform signer's ATA)
        let fee_dest_ata =
            get_associated_token_address(&self.fee_destination.pubkey(), &self.usdc_mint);

        let disc = anchor_discriminator("settle");
        let mut data = Vec::new();
        data.extend_from_slice(&disc);
        data.extend_from_slice(&expected_sequence.to_le_bytes());
        // Vec<u64>: 4-byte LE length prefix + elements
        data.extend_from_slice(&(seller_amounts.len() as u32).to_le_bytes());
        for &amt in seller_amounts {
            data.extend_from_slice(&amt.to_le_bytes());
        }
        data.extend_from_slice(&platform_fee.to_le_bytes());

        let mut accounts = vec![
            AccountMeta::new_readonly(self.config_pda, false), // config PDA
            AccountMeta::new_readonly(self.platform_keypair.pubkey(), true),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(vault_token_pda, false),
            AccountMeta::new(fee_dest_ata, false), // fee_destination's ATA
            AccountMeta::new_readonly(self.usdc_mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false),
        ];

        // remaining_accounts: seller token accounts
        for seller_pk in seller_pubkeys {
            let seller_ata = get_associated_token_address(seller_pk, &self.usdc_mint);
            accounts.push(AccountMeta::new(seller_ata, false));
        }

        let ix = Instruction {
            program_id: self.program_id,
            accounts,
            data,
        };

        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.platform_keypair.pubkey()),
            &[&self.platform_keypair],
            self.svm.latest_blockhash(),
        );

        self.svm.send_transaction(tx).map(|_| ())
    }

    /// Settle with a custom signer (for unauthorized tests).
    fn settle_with_signer(
        &mut self,
        signer: &Keypair,
        buyer_pubkey: &Address,
        expected_sequence: u64,
        seller_pubkeys: &[Address],
        seller_amounts: &[u64],
        platform_fee: u64,
    ) -> Result<(), litesvm::types::FailedTransactionMetadata> {
        let (vault_pda, _) = self.get_vault_pda(buyer_pubkey);
        let (vault_token_pda, _) = self.get_vault_token_pda(buyer_pubkey);
        // Still use fee_destination's ATA (config validates platform = config.authority)
        let fee_dest_ata =
            get_associated_token_address(&self.fee_destination.pubkey(), &self.usdc_mint);

        let disc = anchor_discriminator("settle");
        let mut data = Vec::new();
        data.extend_from_slice(&disc);
        data.extend_from_slice(&expected_sequence.to_le_bytes());
        data.extend_from_slice(&(seller_amounts.len() as u32).to_le_bytes());
        for &amt in seller_amounts {
            data.extend_from_slice(&amt.to_le_bytes());
        }
        data.extend_from_slice(&platform_fee.to_le_bytes());

        let mut accounts = vec![
            AccountMeta::new_readonly(self.config_pda, false), // config PDA
            AccountMeta::new_readonly(signer.pubkey(), true), // wrong signer
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(vault_token_pda, false),
            AccountMeta::new(fee_dest_ata, false),
            AccountMeta::new_readonly(self.usdc_mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false),
        ];

        for seller_pk in seller_pubkeys {
            let seller_ata = get_associated_token_address(seller_pk, &self.usdc_mint);
            accounts.push(AccountMeta::new(seller_ata, false));
        }

        let ix = Instruction {
            program_id: self.program_id,
            accounts,
            data,
        };

        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&signer.pubkey()),
            &[signer],
            self.svm.latest_blockhash(),
        );

        self.svm.send_transaction(tx).map(|_| ())
    }

    /// Builds and sends a withdraw instruction (requires platform co-sign).
    fn withdraw(
        &mut self,
        buyer: &Keypair,
        amount: u64,
    ) -> Result<(), litesvm::types::FailedTransactionMetadata> {
        let (vault_pda, _) = self.get_vault_pda(&buyer.pubkey());
        let (vault_token_pda, _) = self.get_vault_token_pda(&buyer.pubkey());
        let buyer_ata = get_associated_token_address(&buyer.pubkey(), &self.usdc_mint);

        let disc = anchor_discriminator("withdraw");
        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&disc);
        data.extend_from_slice(&amount.to_le_bytes());

        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.config_pda, false), // config PDA
                AccountMeta::new(buyer.pubkey(), true),
                AccountMeta::new_readonly(self.platform_keypair.pubkey(), true), // platform co-signer
                AccountMeta::new(vault_pda, false),
                AccountMeta::new(vault_token_pda, false),
                AccountMeta::new(buyer_ata, false),
                AccountMeta::new_readonly(self.usdc_mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data,
        };

        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&buyer.pubkey()),
            &[buyer, &self.platform_keypair], // TWO signers
            self.svm.latest_blockhash(),
        );

        self.svm.send_transaction(tx).map(|_| ())
    }

    /// Builds and sends a timeout_reclaim instruction.
    fn timeout_reclaim(
        &mut self,
        buyer_pubkey: &Address,
        payer: &Keypair,
    ) -> Result<(), litesvm::types::FailedTransactionMetadata> {
        let (vault_pda, _) = self.get_vault_pda(buyer_pubkey);
        let (vault_token_pda, _) = self.get_vault_token_pda(buyer_pubkey);
        let buyer_ata = get_associated_token_address(buyer_pubkey, &self.usdc_mint);

        let disc = anchor_discriminator("timeout_reclaim");
        let data = disc.to_vec();

        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.config_pda, false), // config PDA
                AccountMeta::new_readonly(*buyer_pubkey, false),
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(vault_pda, false),
                AccountMeta::new(vault_token_pda, false),
                AccountMeta::new(buyer_ata, false),
                AccountMeta::new_readonly(self.usdc_mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data,
        };

        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&payer.pubkey()),
            &[payer],
            self.svm.latest_blockhash(),
        );

        self.svm.send_transaction(tx).map(|_| ())
    }

    /// Calls update_config instruction.
    fn update_config(
        &mut self,
        admin: &Keypair,
        new_admin: Option<Address>,
        new_authority: Option<Address>,
        new_fee_destination: Option<Address>,
        new_fee_bps: Option<u16>,
        new_timeout_seconds: Option<i64>,
        new_paused: Option<bool>,
    ) -> Result<(), litesvm::types::FailedTransactionMetadata> {
        let disc = anchor_discriminator("update_config");
        let mut data = Vec::new();
        data.extend_from_slice(&disc);

        // Borsh Option<Pubkey>: 0 = None, 1 + 32 bytes = Some
        fn encode_option_pubkey(data: &mut Vec<u8>, val: Option<Address>) {
            match val {
                None => data.push(0),
                Some(pk) => { data.push(1); data.extend_from_slice(pk.as_ref()); }
            }
        }
        fn encode_option_u16(data: &mut Vec<u8>, val: Option<u16>) {
            match val {
                None => data.push(0),
                Some(v) => { data.push(1); data.extend_from_slice(&v.to_le_bytes()); }
            }
        }
        fn encode_option_i64(data: &mut Vec<u8>, val: Option<i64>) {
            match val {
                None => data.push(0),
                Some(v) => { data.push(1); data.extend_from_slice(&v.to_le_bytes()); }
            }
        }
        fn encode_option_bool(data: &mut Vec<u8>, val: Option<bool>) {
            match val {
                None => data.push(0),
                Some(v) => { data.push(1); data.push(v as u8); }
            }
        }

        encode_option_pubkey(&mut data, new_admin);
        encode_option_pubkey(&mut data, new_authority);
        encode_option_pubkey(&mut data, new_fee_destination);
        encode_option_u16(&mut data, new_fee_bps);
        encode_option_i64(&mut data, new_timeout_seconds);
        encode_option_bool(&mut data, new_paused);

        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(admin.pubkey(), true),
                AccountMeta::new(self.config_pda, false),
            ],
            data,
        };

        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&admin.pubkey()),
            &[admin],
            self.svm.latest_blockhash(),
        );

        self.svm.send_transaction(tx).map(|_| ())
    }

    /// Reads and deserializes a BuyerVault account.
    fn get_vault(&self, buyer: &Address) -> VaultState {
        let (vault_pda, _) = self.get_vault_pda(buyer);
        let account = self.svm.get_account(&vault_pda).expect("Vault account not found");
        // Anchor accounts: 8-byte discriminator + data
        let data = &account.data[8..];
        VaultState {
            buyer: Address::try_from(&data[0..32]).unwrap(),
            sequence: u64::from_le_bytes(data[32..40].try_into().unwrap()),
            last_settled_at: i64::from_le_bytes(data[40..48].try_into().unwrap()),
            bump: data[48],
            token_bump: data[49],
        }
    }

    /// Reads and deserializes a Config account.
    fn get_config(&self) -> ConfigState {
        let account = self.svm.get_account(&self.config_pda).expect("Config account not found");
        let data = &account.data[8..]; // skip 8-byte discriminator
        ConfigState {
            admin: Address::try_from(&data[0..32]).unwrap(),
            authority: Address::try_from(&data[32..64]).unwrap(),
            fee_destination: Address::try_from(&data[64..96]).unwrap(),
            usdc_mint: Address::try_from(&data[96..128]).unwrap(),
            fee_bps: u16::from_le_bytes(data[128..130].try_into().unwrap()),
            timeout_seconds: i64::from_le_bytes(data[130..138].try_into().unwrap()),
            paused: data[138] != 0,
            bump: data[139],
        }
    }

    /// Reads a token account balance.
    fn get_token_balance(&self, token_account: &Address) -> u64 {
        let account = self
            .svm
            .get_account(token_account)
            .expect("Token account not found");
        let token_state = SplTokenAccount::unpack(&account.data).unwrap();
        token_state.amount
    }

    /// Advances the clock by the given number of seconds.
    fn warp_time(&mut self, seconds: i64) {
        let mut clock: Clock = self.svm.get_sysvar();
        clock.unix_timestamp += seconds;
        self.svm.set_sysvar(&clock);
    }

    /// Returns a fresh copy of the platform keypair (Keypair doesn't impl Clone).
    fn platform_keypair_copy(&self) -> Keypair {
        test_platform_keypair()
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct VaultState {
    buyer: Address,
    sequence: u64,
    last_settled_at: i64,
    bump: u8,
    token_bump: u8,
}

#[derive(Debug)]
#[allow(dead_code)]
struct ConfigState {
    admin: Address,
    authority: Address,
    fee_destination: Address,
    usdc_mint: Address,
    fee_bps: u16,
    timeout_seconds: i64,
    paused: bool,
    bump: u8,
}

/// Checks if an error is a specific Anchor custom error.
/// Anchor custom error codes start at 6000.
fn is_anchor_error(
    err: &litesvm::types::FailedTransactionMetadata,
    error_offset: u32,
) -> bool {
    let expected_code = 6000 + error_offset;
    matches!(
        &err.err,
        solana_transaction_error::TransactionError::InstructionError(
            _,
            solana_instruction::error::InstructionError::Custom(code),
        ) if *code == expected_code
    )
}

// Error offsets for EscrowError variants (order in error.rs)
const ERR_ZERO_AMOUNT: u32 = 0;
const ERR_INSUFFICIENT_BALANCE: u32 = 1;
const ERR_INVALID_SEQUENCE: u32 = 2;
const ERR_ARITHMETIC_OVERFLOW: u32 = 3;
const ERR_SELLER_COUNT_MISMATCH: u32 = 4;
const ERR_TIMEOUT_NOT_REACHED: u32 = 5;
const ERR_VAULT_EMPTY: u32 = 6;
const ERR_PAUSED: u32 = 7;
const ERR_UNAUTHORIZED_ADMIN: u32 = 8;

// ============================================================================
// CONFIG TESTS
// ============================================================================

#[test]
fn test_initialize_config() {
    let env = TestEnv::new();
    let config = env.get_config();

    assert_eq!(config.admin, env.platform_keypair.pubkey());
    assert_eq!(config.authority, env.platform_keypair.pubkey());
    assert_eq!(config.fee_destination, env.fee_destination.pubkey());
    assert_eq!(config.usdc_mint, env.usdc_mint);
    assert_eq!(config.fee_bps, PLATFORM_FEE_BPS);
    assert_eq!(config.timeout_seconds, TIMEOUT_SECONDS);
    assert!(!config.paused);
}

#[test]
fn test_initialize_config_double_init_fails() {
    let mut env = TestEnv::new();
    // Config was already initialized in TestEnv::new(), second call should fail
    let result = env.initialize_config();
    assert!(result.is_err());
}

#[test]
fn test_update_config_admin_can_update() {
    let mut env = TestEnv::new();

    let new_fee_bps: u16 = 300;
    env.update_config(
        // admin = platform_keypair (set during init)
        &env.platform_keypair_copy(),
        None, None, None,
        Some(new_fee_bps),
        None, None,
    ).unwrap();

    let config = env.get_config();
    assert_eq!(config.fee_bps, 300);
}

#[test]
fn test_update_config_non_admin_fails() {
    let mut env = TestEnv::new();

    let fake_admin = Keypair::new();
    env.svm.airdrop(&fake_admin.pubkey(), 10_000_000_000).unwrap();

    let result = env.update_config(
        &fake_admin,
        None, None, None, Some(100), None, None,
    );
    assert!(result.is_err());
    assert!(is_anchor_error(&result.unwrap_err(), ERR_UNAUTHORIZED_ADMIN));
}

#[test]
fn test_update_config_admin_transfer() {
    let mut env = TestEnv::new();

    let new_admin = Keypair::new();
    env.svm.airdrop(&new_admin.pubkey(), 10_000_000_000).unwrap();

    // Transfer admin to new_admin
    env.update_config(
        &env.platform_keypair_copy(),
        Some(new_admin.pubkey()), None, None, None, None, None,
    ).unwrap();

    let config = env.get_config();
    assert_eq!(config.admin, new_admin.pubkey());

    // Old admin can no longer update
    let result = env.update_config(
        &env.platform_keypair_copy(),
        None, None, None, Some(100), None, None,
    );
    assert!(result.is_err());
    assert!(is_anchor_error(&result.unwrap_err(), ERR_UNAUTHORIZED_ADMIN));

    // New admin can update
    env.update_config(
        &new_admin,
        None, None, None, Some(200), None, None,
    ).unwrap();
    assert_eq!(env.get_config().fee_bps, 200);
}

// ============================================================================
// PAUSE TESTS
// ============================================================================

#[test]
fn test_pause_blocks_deposit() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);

    // Pause
    env.update_config(
        &env.platform_keypair_copy(),
        None, None, None, None, None, Some(true),
    ).unwrap();

    let result = env.deposit(&buyer, 500_000);
    assert!(result.is_err());
    assert!(is_anchor_error(&result.unwrap_err(), ERR_PAUSED));
}

#[test]
fn test_pause_blocks_settle() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();
    let seller = env.create_seller();

    // Pause
    env.update_config(
        &env.platform_keypair_copy(),
        None, None, None, None, None, Some(true),
    ).unwrap();

    let result = env.settle(&buyer.pubkey(), 1, &[seller.pubkey()], &[950_000], 50_000);
    assert!(result.is_err());
    assert!(is_anchor_error(&result.unwrap_err(), ERR_PAUSED));
}

#[test]
fn test_pause_blocks_withdraw() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    // Pause
    env.update_config(
        &env.platform_keypair_copy(),
        None, None, None, None, None, Some(true),
    ).unwrap();

    let result = env.withdraw(&buyer, 500_000);
    assert!(result.is_err());
    assert!(is_anchor_error(&result.unwrap_err(), ERR_PAUSED));
}

#[test]
fn test_pause_does_not_block_timeout_reclaim() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    // Pause
    env.update_config(
        &env.platform_keypair_copy(),
        None, None, None, None, None, Some(true),
    ).unwrap();

    env.warp_time(TIMEOUT_SECONDS + 1);

    let payer = Keypair::new();
    env.svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();

    // timeout_reclaim should still work when paused (buyer protection)
    env.timeout_reclaim(&buyer.pubkey(), &payer).unwrap();

    let buyer_ata = get_associated_token_address(&buyer.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&buyer_ata), 1_000_000);
}

#[test]
fn test_unpause_restores_operations() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);

    // Pause then unpause
    env.update_config(
        &env.platform_keypair_copy(),
        None, None, None, None, None, Some(true),
    ).unwrap();
    env.svm.expire_blockhash();
    env.update_config(
        &env.platform_keypair_copy(),
        None, None, None, None, None, Some(false),
    ).unwrap();

    // Deposit should work again
    env.deposit(&buyer, 500_000).unwrap();

    let (vault_token_pda, _) = env.get_vault_token_pda(&buyer.pubkey());
    assert_eq!(env.get_token_balance(&vault_token_pda), 500_000);
}

// ============================================================================
// FEE DESTINATION TESTS
// ============================================================================

#[test]
fn test_settle_pays_fee_destination_not_authority() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let seller = env.create_seller();

    env.settle(
        &buyer.pubkey(), 1,
        &[seller.pubkey()], &[950_000], 50_000,
    ).unwrap();

    // Fees should go to fee_destination's ATA
    let fee_dest_ata = get_associated_token_address(&env.fee_destination.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&fee_dest_ata), 50_000);
}

// ============================================================================
// DEPOSIT TESTS
// ============================================================================

#[test]
fn test_deposit_creates_vault() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);

    env.deposit(&buyer, 500_000).unwrap();

    let vault = env.get_vault(&buyer.pubkey());
    assert_eq!(vault.buyer, buyer.pubkey());
    assert_eq!(vault.sequence, 0);
    assert!(vault.last_settled_at > 0);

    let (vault_token_pda, _) = env.get_vault_token_pda(&buyer.pubkey());
    let balance = env.get_token_balance(&vault_token_pda);
    assert_eq!(balance, 500_000);
}

#[test]
fn test_deposit_subsequent_increments_balance() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(2_000_000);

    env.deposit(&buyer, 500_000).unwrap();
    env.deposit(&buyer, 300_000).unwrap();

    let vault = env.get_vault(&buyer.pubkey());
    assert_eq!(vault.buyer, buyer.pubkey());
    assert_eq!(vault.sequence, 0);

    let (vault_token_pda, _) = env.get_vault_token_pda(&buyer.pubkey());
    let balance = env.get_token_balance(&vault_token_pda);
    assert_eq!(balance, 800_000);
}

#[test]
fn test_deposit_zero_amount_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);

    let result = env.deposit(&buyer, 0);
    assert!(result.is_err());
    assert!(is_anchor_error(&result.unwrap_err(), ERR_ZERO_AMOUNT));
}

#[test]
fn test_deposit_insufficient_buyer_balance_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(100_000);

    let result = env.deposit(&buyer, 500_000);
    assert!(result.is_err());
}

// ============================================================================
// SETTLE TESTS
// ============================================================================

#[test]
fn test_settle_single_seller() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let seller = env.create_seller();
    let seller_ata = get_associated_token_address(&seller.pubkey(), &env.usdc_mint);

    // 950k to seller (95%), 50k platform fee (5%) = 1M total
    env.settle(
        &buyer.pubkey(),
        1,
        &[seller.pubkey()],
        &[950_000],
        50_000,
    )
    .unwrap();

    let vault = env.get_vault(&buyer.pubkey());
    assert_eq!(vault.sequence, 1);

    assert_eq!(env.get_token_balance(&seller_ata), 950_000);

    let fee_dest_ata =
        get_associated_token_address(&env.fee_destination.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&fee_dest_ata), 50_000);

    let (vault_token_pda, _) = env.get_vault_token_pda(&buyer.pubkey());
    assert_eq!(env.get_token_balance(&vault_token_pda), 9_000_000);
}

#[test]
fn test_settle_multiple_sellers() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let seller1 = env.create_seller();
    let seller2 = env.create_seller();
    let seller3 = env.create_seller();

    let seller1_ata = get_associated_token_address(&seller1.pubkey(), &env.usdc_mint);
    let seller2_ata = get_associated_token_address(&seller2.pubkey(), &env.usdc_mint);
    let seller3_ata = get_associated_token_address(&seller3.pubkey(), &env.usdc_mint);

    env.settle(
        &buyer.pubkey(),
        1,
        &[seller1.pubkey(), seller2.pubkey(), seller3.pubkey()],
        &[1_000_000, 2_000_000, 500_000],
        184_210,
    )
    .unwrap();

    assert_eq!(env.get_token_balance(&seller1_ata), 1_000_000);
    assert_eq!(env.get_token_balance(&seller2_ata), 2_000_000);
    assert_eq!(env.get_token_balance(&seller3_ata), 500_000);

    let fee_dest_ata =
        get_associated_token_address(&env.fee_destination.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&fee_dest_ata), 184_210);
}

#[test]
fn test_settle_updates_sequence() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let seller = env.create_seller();

    env.settle(&buyer.pubkey(), 1, &[seller.pubkey()], &[100_000], 5_000)
        .unwrap();
    assert_eq!(env.get_vault(&buyer.pubkey()).sequence, 1);

    env.settle(&buyer.pubkey(), 2, &[seller.pubkey()], &[100_000], 5_000)
        .unwrap();
    assert_eq!(env.get_vault(&buyer.pubkey()).sequence, 2);
}

#[test]
fn test_settle_updates_last_settled_at() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let vault_before = env.get_vault(&buyer.pubkey());
    let initial_timestamp = vault_before.last_settled_at;

    env.warp_time(100);

    let seller = env.create_seller();
    env.settle(&buyer.pubkey(), 1, &[seller.pubkey()], &[100_000], 5_000)
        .unwrap();

    let vault_after = env.get_vault(&buyer.pubkey());
    assert!(vault_after.last_settled_at > initial_timestamp);
}

#[test]
fn test_settle_wrong_sequence_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let seller = env.create_seller();

    let result = env.settle(&buyer.pubkey(), 5, &[seller.pubkey()], &[100_000], 5_000);
    assert!(result.is_err());
    assert!(is_anchor_error(&result.unwrap_err(), ERR_INVALID_SEQUENCE));
}

#[test]
fn test_settle_replay_same_sequence_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let seller = env.create_seller();

    env.settle(&buyer.pubkey(), 1, &[seller.pubkey()], &[100_000], 5_000)
        .unwrap();

    // Expire the blockhash to avoid AlreadyProcessed dedup
    env.svm.expire_blockhash();

    let result = env.settle(&buyer.pubkey(), 1, &[seller.pubkey()], &[100_000], 5_000);
    assert!(result.is_err());
    assert!(is_anchor_error(&result.unwrap_err(), ERR_INVALID_SEQUENCE));
}

#[test]
fn test_settle_unauthorized_signer_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let seller = env.create_seller();

    let fake_platform = Keypair::new();
    env.svm
        .airdrop(&fake_platform.pubkey(), 10_000_000_000)
        .unwrap();

    let result = env.settle_with_signer(
        &fake_platform,
        &buyer.pubkey(),
        1,
        &[seller.pubkey()],
        &[100_000],
        5_000,
    );
    assert!(result.is_err());
}

#[test]
fn test_settle_insufficient_balance_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    let seller = env.create_seller();

    let result = env.settle(
        &buyer.pubkey(),
        1,
        &[seller.pubkey()],
        &[2_000_000],
        100_000,
    );
    assert!(result.is_err());
    assert!(is_anchor_error(
        &result.unwrap_err(),
        ERR_INSUFFICIENT_BALANCE
    ));
}

#[test]
fn test_settle_seller_count_mismatch_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let seller1 = env.create_seller();
    let seller2 = env.create_seller();

    // 2 seller accounts but 3 amounts
    let result = env.settle(
        &buyer.pubkey(),
        1,
        &[seller1.pubkey(), seller2.pubkey()],
        &[100_000, 200_000, 300_000],
        30_000,
    );
    assert!(result.is_err());
    assert!(is_anchor_error(
        &result.unwrap_err(),
        ERR_SELLER_COUNT_MISMATCH
    ));
}

#[test]
fn test_settle_empty_sellers_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let result = env.settle(&buyer.pubkey(), 1, &[], &[], 50_000);
    assert!(result.is_err());
    assert!(is_anchor_error(&result.unwrap_err(), ERR_ZERO_AMOUNT));
}

// ============================================================================
// WITHDRAW TESTS
// ============================================================================

#[test]
fn test_withdraw_partial() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    env.withdraw(&buyer, 400_000).unwrap();

    let (vault_token_pda, _) = env.get_vault_token_pda(&buyer.pubkey());
    assert_eq!(env.get_token_balance(&vault_token_pda), 600_000);

    let buyer_ata = get_associated_token_address(&buyer.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&buyer_ata), 400_000);
}

#[test]
fn test_withdraw_full_balance() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    env.withdraw(&buyer, 1_000_000).unwrap();

    let (vault_token_pda, _) = env.get_vault_token_pda(&buyer.pubkey());
    assert_eq!(env.get_token_balance(&vault_token_pda), 0);

    let buyer_ata = get_associated_token_address(&buyer.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&buyer_ata), 1_000_000);
}

#[test]
fn test_withdraw_zero_amount_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    let result = env.withdraw(&buyer, 0);
    assert!(result.is_err());
    assert!(is_anchor_error(&result.unwrap_err(), ERR_ZERO_AMOUNT));
}

#[test]
fn test_withdraw_exceeds_balance_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    let result = env.withdraw(&buyer, 2_000_000);
    assert!(result.is_err());
    assert!(is_anchor_error(
        &result.unwrap_err(),
        ERR_INSUFFICIENT_BALANCE
    ));
}

#[test]
fn test_withdraw_wrong_buyer_fails() {
    let mut env = TestEnv::new();
    let buyer_a = env.create_buyer(1_000_000);
    env.deposit(&buyer_a, 1_000_000).unwrap();

    let buyer_b = env.create_buyer(0);

    // buyer_b tries to withdraw from buyer_a's vault
    let (vault_pda, _) = env.get_vault_pda(&buyer_a.pubkey());
    let (vault_token_pda, _) = env.get_vault_token_pda(&buyer_a.pubkey());
    let buyer_b_ata = get_associated_token_address(&buyer_b.pubkey(), &env.usdc_mint);

    let disc = anchor_discriminator("withdraw");
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&disc);
    data.extend_from_slice(&500_000u64.to_le_bytes());

    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(env.config_pda, false), // config PDA
            AccountMeta::new(buyer_b.pubkey(), true),
            AccountMeta::new_readonly(env.platform_keypair.pubkey(), true), // platform co-signer
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(vault_token_pda, false),
            AccountMeta::new(buyer_b_ata, false),
            AccountMeta::new_readonly(env.usdc_mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data,
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&buyer_b.pubkey()),
        &[&buyer_b, &env.platform_keypair], // Add platform as signer
        env.svm.latest_blockhash(),
    );

    let result = env.svm.send_transaction(tx);
    assert!(result.is_err());
}

#[test]
fn test_withdraw_without_platform_cosign_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    // Build withdraw instruction WITHOUT platform co-signer
    let (vault_pda, _) = env.get_vault_pda(&buyer.pubkey());
    let (vault_token_pda, _) = env.get_vault_token_pda(&buyer.pubkey());
    let buyer_ata = get_associated_token_address(&buyer.pubkey(), &env.usdc_mint);

    let disc = anchor_discriminator("withdraw");
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&disc);
    data.extend_from_slice(&500_000u64.to_le_bytes());

    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(env.config_pda, false), // config PDA
            AccountMeta::new(buyer.pubkey(), true),
            // NO platform account -- this is the attack vector
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(vault_token_pda, false),
            AccountMeta::new(buyer_ata, false),
            AccountMeta::new_readonly(env.usdc_mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data,
    };

    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&buyer.pubkey()),
        &[&buyer],
        env.svm.latest_blockhash(),
    );

    let result = env.svm.send_transaction(tx);
    assert!(result.is_err(), "Withdraw without platform co-sign must fail");
}

// ============================================================================
// TIMEOUT RECLAIM TESTS
// ============================================================================

#[test]
fn test_timeout_reclaim_after_timeout() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    let buyer_ata = get_associated_token_address(&buyer.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&buyer_ata), 0);

    env.warp_time(TIMEOUT_SECONDS + 1);

    let payer = Keypair::new();
    env.svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();

    env.timeout_reclaim(&buyer.pubkey(), &payer).unwrap();

    assert_eq!(env.get_token_balance(&buyer_ata), 1_000_000);

    let (vault_token_pda, _) = env.get_vault_token_pda(&buyer.pubkey());
    assert_eq!(env.get_token_balance(&vault_token_pda), 0);
}

#[test]
fn test_timeout_reclaim_before_timeout_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    let payer = Keypair::new();
    env.svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();

    let result = env.timeout_reclaim(&buyer.pubkey(), &payer);
    assert!(result.is_err());
    assert!(is_anchor_error(
        &result.unwrap_err(),
        ERR_TIMEOUT_NOT_REACHED
    ));
}

#[test]
fn test_timeout_reclaim_permissionless() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    env.warp_time(TIMEOUT_SECONDS + 1);

    let random_party = Keypair::new();
    env.svm
        .airdrop(&random_party.pubkey(), 10_000_000_000)
        .unwrap();

    env.timeout_reclaim(&buyer.pubkey(), &random_party).unwrap();

    let buyer_ata = get_associated_token_address(&buyer.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&buyer_ata), 1_000_000);
}

#[test]
fn test_timeout_reclaim_empty_vault_fails() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(1_000_000);
    env.deposit(&buyer, 1_000_000).unwrap();

    env.withdraw(&buyer, 1_000_000).unwrap();

    env.warp_time(TIMEOUT_SECONDS + 1);

    let payer = Keypair::new();
    env.svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();

    let result = env.timeout_reclaim(&buyer.pubkey(), &payer);
    assert!(result.is_err());
    assert!(is_anchor_error(&result.unwrap_err(), ERR_VAULT_EMPTY));
}

// ============================================================================
// OVERFLOW EDGE CASES
// ============================================================================

#[test]
fn test_settle_overflow_seller_amounts() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let seller1 = env.create_seller();
    let seller2 = env.create_seller();

    let result = env.settle(
        &buyer.pubkey(),
        1,
        &[seller1.pubkey(), seller2.pubkey()],
        &[u64::MAX, 1],
        0,
    );
    assert!(result.is_err());
    assert!(is_anchor_error(
        &result.unwrap_err(),
        ERR_ARITHMETIC_OVERFLOW
    ));
}

#[test]
fn test_settle_overflow_total_with_fee() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let seller = env.create_seller();

    let result = env.settle(
        &buyer.pubkey(),
        1,
        &[seller.pubkey()],
        &[u64::MAX],
        1,
    );
    assert!(result.is_err());
    assert!(is_anchor_error(
        &result.unwrap_err(),
        ERR_ARITHMETIC_OVERFLOW
    ));
}

// ============================================================================
// INTEGRATION / LIFECYCLE TESTS
// ============================================================================

#[test]
fn test_full_lifecycle() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(5_000_000);
    env.deposit(&buyer, 5_000_000).unwrap();

    let seller1 = env.create_seller();
    let seller2 = env.create_seller();

    // Settlement 1: 950k to each seller + 100k platform fee = 2M total
    env.settle(
        &buyer.pubkey(),
        1,
        &[seller1.pubkey(), seller2.pubkey()],
        &[950_000, 950_000],
        100_000,
    )
    .unwrap();

    let (vault_token_pda, _) = env.get_vault_token_pda(&buyer.pubkey());
    assert_eq!(env.get_token_balance(&vault_token_pda), 3_000_000);

    let seller1_ata = get_associated_token_address(&seller1.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&seller1_ata), 950_000);

    let seller2_ata = get_associated_token_address(&seller2.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&seller2_ata), 950_000);

    // Withdraw remainder
    env.withdraw(&buyer, 3_000_000).unwrap();

    assert_eq!(env.get_token_balance(&vault_token_pda), 0);
    let buyer_ata = get_associated_token_address(&buyer.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&buyer_ata), 3_000_000);
}

#[test]
fn test_multiple_settlements_sequential() {
    let mut env = TestEnv::new();
    let buyer = env.create_buyer(10_000_000);
    env.deposit(&buyer, 10_000_000).unwrap();

    let seller = env.create_seller();

    env.settle(&buyer.pubkey(), 1, &[seller.pubkey()], &[100_000], 5_000)
        .unwrap();
    assert_eq!(env.get_vault(&buyer.pubkey()).sequence, 1);

    env.settle(&buyer.pubkey(), 2, &[seller.pubkey()], &[200_000], 10_000)
        .unwrap();
    assert_eq!(env.get_vault(&buyer.pubkey()).sequence, 2);

    env.settle(&buyer.pubkey(), 3, &[seller.pubkey()], &[300_000], 15_000)
        .unwrap();
    assert_eq!(env.get_vault(&buyer.pubkey()).sequence, 3);

    let seller_ata = get_associated_token_address(&seller.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&seller_ata), 600_000);

    let fee_dest_ata =
        get_associated_token_address(&env.fee_destination.pubkey(), &env.usdc_mint);
    assert_eq!(env.get_token_balance(&fee_dest_ata), 30_000);

    let (vault_token_pda, _) = env.get_vault_token_pda(&buyer.pubkey());
    assert_eq!(env.get_token_balance(&vault_token_pda), 9_370_000);
}
