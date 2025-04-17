use anchor_lang::prelude::*;
use anchor_spl::token::{ self, Token, Transfer, Mint, TokenAccount };
use anchor_lang::solana_program::pubkey::Pubkey;
use anchor_lang::solana_program::{ system_instruction, program::invoke, hash }; // Этот импорт уже правильный, т.к. он из anchor_lang

declare_id!("39yLJnQTeKW56z8kK64KReuEHVKiHLGGr3ZKTM6dLJrS");

/// Treasury wallet address for collecting SOL fees.
const TREASURY_SOL_PUBKEY: Pubkey = pubkey!("DDx7B6zkNhseqcp8Ym5JnP6YyRtMJ19cAML7EtfNz3CX");
/// Fee amount for creating a Vault in SOL lamports (e.g., 0.5 SOL).
const CREATE_VAULT_FEE_SOL_LAMPORTS: u64 = 500_000_000;

/// Represents a single instance of liquidity provided by a user.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct LiquidityProvision {
    /// The public key of the liquidity provider.
    pub provider: Pubkey,
    /// The amount of liquidity provided in this instance.
    pub amount: u64,
    /// Unix timestamp when the liquidity was provided.
    pub timestamp: i64,
    /// Flag indicating if this specific provision has been withdrawn.
    pub withdrawn: bool,
}

/// Accounts required for initializing the player's betting account for the game session.
#[derive(Accounts)]
pub struct InitializePlayerBets<'info> {
    /// The player initializing their bets account (signer). Pays for account creation.
    #[account(mut)]
    pub player: Signer<'info>,

    /// The global game session account. Used for PDA seeds.
    #[account(seeds = [b"game_session"], bump = game_session.bump)]
    pub game_session: Account<'info, GameSession>,

    /// The PlayerBets PDA account to be initialized.
    /// Seeds: [b"player_bets", game_session_key, player_key]
    #[account(
        init,
        payer = player,
        space = 8 + 32 + 8 + 32 + 32 + (4 + std::mem::size_of::<Bet>() * MAX_BETS_PER_ROUND) + 1,
        seeds = [b"player_bets", game_session.key().as_ref(), player.key().as_ref()],
        bump
    )]
    pub player_bets: Account<'info, PlayerBets>,

    /// The Solana System Program, required for account creation.
    pub system_program: Program<'info, System>,
    /// The Rent Sysvar, required for account initialization checks.
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct ClosePlayerBetsAccount<'info> {
    /// The player closing their account (signer). Rent SOL will be returned here.
    #[account(mut)]
    pub player: Signer<'info>,

    /// The PlayerBets PDA account to be closed.
    /// Seeds: [b"player_bets", game_session.key().as_ref(), player.key().as_ref()]
    /// The `close` constraint directs Solana to close the account and send the lamports
    /// to the specified `player` account.
    #[account(
        mut, // Account data will be wiped, and lamports transferred.
        seeds = [b"player_bets", game_session.key().as_ref(), player.key().as_ref()],
        bump = player_bets.bump, // Make sure we are closing the correct PDA
        close = player // Return lamports to the player signer.
    )]
    pub player_bets: Account<'info, PlayerBets>,

    /// The global game session account. Needed ONLY for deriving the player_bets PDA seeds.
    /// It's not modified.
    #[account(seeds = [b"game_session"], bump = game_session.bump)]
    pub game_session: Account<'info, GameSession>,
    // SystemProgram is implicitly used by Anchor for the `close` operation.
}

// Max bets stored per player per round in their PlayerBets account.
const MAX_BETS_PER_ROUND: usize = 10; // Example limit for space calculation

/// Represents a single bet placed by a player.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct Bet {
    /// The amount wagered for this bet (in the vault token's smallest unit, e.g., lamports).
    pub amount: u64,
    /// The type of bet placed, encoded as a u8. See BET_TYPE_* constants.
    pub bet_type: u8,
    /// Numbers associated with the bet. Usage depends on `bet_type`.
    /// Unused elements are typically 0. Max 4 numbers needed (e.g., for Corner bets).
    pub numbers: [u8; 4],
}

/// Tracks the accumulated reward for a specific liquidity provider within a vault.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct ProviderReward {
    /// The public key of the liquidity provider.
    pub provider: Pubkey,
    /// The total reward accumulated by this provider, claimable via `withdraw_provider_revenue`.
    pub accumulated_reward: u64,
}

/// Defines the possible states of a roulette game round.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Default)]
pub enum RoundStatus {
    #[default]
    NotStarted,
    AcceptingBets,
    BetsClosed,
    Completed,
}

/// Enum defining the various types of bets possible in roulette.
/// Note: This enum is used internally for logic but bets are stored using u8 `bet_type` in the `Bet` struct.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq)]
pub enum BetType {
    Straight {
        number: u8,
    },
    Split {
        first: u8,
        second: u8,
    },
    Corner {
        top_left: u8,
    },
    Street {
        street: u8,
    },
    SixLine {
        six_line: u8,
    },
    FirstFour,
    Red,
    Black,
    Even,
    Odd,
    Manque, // 1-18
    Passe, // 19-36
    Column {
        column: u8,
    },
    P12, // 1-12
    M12, // 13-24
    D12, // 25-36
}

/// Accounts required for initializing a new `VaultAccount` for a specific SPL token mint.
/// This creates the central liquidity pool and reward tracking for a token.
#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Used for PDA seeds. Validity checked indirectly via vault_token_account.
    pub token_mint: AccountInfo<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + std::mem::size_of::<VaultAccount>() + 4096
    )] // Consider more precise space calculation
    pub vault: Account<'info, VaultAccount>,

    /// CHECK: Deserialized and checked (mint, owner) manually in the instruction.
    pub vault_token_account: AccountInfo<'info>,

    // --- Account for collecting SOL fee ---
    /// CHECK: Address checked in instruction logic, used for SOL transfer. Must be writable.
    #[account(
        mut,
        address = TREASURY_SOL_PUBKEY
    )]
    pub treasury_sol_account: AccountInfo<'info>,

    // System programs
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

/// Accounts required for initializing the global `GameSession` account.
#[derive(Accounts)]
pub struct InitializeGameSession<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(init, payer = authority, space = 85, seeds = [b"game_session"], bump)]
    pub game_session: Account<'info, GameSession>,

    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

/// Accounts required for a user to add liquidity to an existing vault.
#[derive(Accounts)]
pub struct ProvideLiquidity<'info> {
    /// The vault account to which liquidity is being added. Mutable to update `total_liquidity` and `liquidity_pool`.
    #[account(mut)]
    pub vault: Account<'info, VaultAccount>,

    /// CHECK: Validated in instruction logic (is TokenAccount).
    #[account(mut)]
    pub provider_token_account: AccountInfo<'info>,

    /// CHECK: Validated in instruction logic (is TokenAccount). Constraint ensures it matches the vault's stored `token_account`.
    #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
    pub vault_token_account: AccountInfo<'info>,

    /// The liquidity provider (signer).
    #[account(mut)]
    pub liquidity_provider: Signer<'info>,

    /// The SPL Token Program, needed for the token transfer CPI.
    pub token_program: Program<'info, Token>,
    /// The Solana System Program.
    pub system_program: Program<'info, System>,
}

/// Accounts required for a liquidity provider to withdraw their *exact* initial liquidity contribution(s).
/// Does not include rewards.
#[derive(Accounts)]
pub struct WithdrawLiquidity<'info> {
    /// The vault account from which liquidity is being withdrawn. Mutable to update `total_liquidity` and `liquidity_pool`.
    #[account(mut)]
    pub vault: Account<'info, VaultAccount>,

    /// CHECK: Validated in instruction logic (is TokenAccount).
    #[account(mut)]
    pub provider_token_account: AccountInfo<'info>,

    /// CHECK: Validated in instruction logic (is TokenAccount). Constraint ensures it matches the vault's stored `token_account`.
    #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
    pub vault_token_account: AccountInfo<'info>,

    /// The liquidity provider requesting the withdrawal (signer).
    #[account(mut)]
    pub liquidity_provider: Signer<'info>,

    /// The SPL Token Program, needed for the token transfer CPI.
    pub token_program: Program<'info, Token>,
    /// The Solana System Program.
    pub system_program: Program<'info, System>,
}

/// Accounts required for a liquidity provider to withdraw their accumulated rewards.
#[derive(Accounts)]
pub struct WithdrawProviderRevenue<'info> {
    /// The vault account holding the rewards. Mutable to update `total_liquidity` and `provider_rewards`.
    #[account(mut)]
    pub vault: Account<'info, VaultAccount>,

    /// CHECK: Validated in instruction logic (is TokenAccount).
    #[account(mut)]
    pub provider_token_account: AccountInfo<'info>,

    /// CHECK: Validated in instruction logic (is TokenAccount). Constraint ensures it matches the vault's stored `token_account`.
    #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
    pub vault_token_account: AccountInfo<'info>,

    /// The liquidity provider requesting the withdrawal (signer).
    #[account(mut)]
    pub liquidity_provider: Signer<'info>,

    /// The SPL Token Program, needed for the token transfer CPI.
    pub token_program: Program<'info, Token>,
    /// The Solana System Program.
    pub system_program: Program<'info, System>,
}

/// Accounts required for the program authority to withdraw accumulated owner revenue.
#[derive(Accounts)]
pub struct WithdrawOwnerRevenue<'info> {
    /// The vault account holding the owner revenue. Mutable to update `total_liquidity` and `owner_reward`.
    #[account(mut)]
    pub vault: Account<'info, VaultAccount>,

    /// CHECK: Validated in instruction logic (is TokenAccount).
    #[account(mut)]
    pub owner_token_account: AccountInfo<'info>,

    /// CHECK: Validated in instruction logic (is TokenAccount). Constraint ensures it matches the vault's stored `token_account`.
    #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
    pub vault_token_account: AccountInfo<'info>,

    /// The program authority requesting the withdrawal (signer). Must match `vault.authority`.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// The SPL Token Program, needed for the token transfer CPI.
    pub token_program: Program<'info, Token>,
    /// The Solana System Program.
    pub system_program: Program<'info, System>,
}

/// Accounts for initializing a vault AND providing initial liquidity in a single transaction.
/// Useful for bootstrapping a new token vault.
#[derive(Accounts)]
pub struct InitializeAndProvideLiquidity<'info> {
    /// The authority account (signer) authorized to initialize vaults. Pays for account creation.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// The mint account of the SPL token for the new vault.
    /// CHECK: Verified in instruction logic (is Mint).
    pub token_mint: AccountInfo<'info>,

    /// The `VaultAccount` PDA to be initialized.
    /// Seeds: [b"vault", token_mint_key]
    #[account(
        init,
        payer = authority,
        space = 8 + std::mem::size_of::<VaultAccount>() + 10000, // Base size + buffer for Vecs
        seeds = [b"vault", token_mint.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, VaultAccount>,

    /// CHECK: Validated in instruction logic (is TokenAccount).
    #[account(mut)]
    pub provider_token_account: AccountInfo<'info>,

    /// CHECK: Verified in instruction logic (is TokenAccount).
    pub vault_token_account: AccountInfo<'info>,

    /// The initial liquidity provider (signer). Can be the same as `authority`.
    #[account(mut)]
    pub liquidity_provider: Signer<'info>,

    // --- Добавить это ---
    /// CHECK: Address checked in instruction logic, used for SOL transfer. Must be writable.
    #[account(
        mut,
        address = TREASURY_SOL_PUBKEY
    )]
    pub treasury_sol_account: AccountInfo<'info>,
    // --- Конец добавления ---

    /// The Solana System Program.
    pub system_program: Program<'info, System>,
    /// The SPL Token Program.
    pub token_program: Program<'info, Token>,
    /// The Rent Sysvar.
    pub rent: Sysvar<'info, Rent>,
}

/// Accounts required to start a new roulette round.
#[derive(Accounts)]
pub struct StartNewRound<'info> {
    /// The global `GameSession` account. Mutable to update round status, number, times, etc.
    #[account(mut, seeds = [b"game_session"], bump = game_session.bump)]
    pub game_session: Account<'info, GameSession>,

    /// The user initiating the new round (signer).
    #[account(mut)]
    pub starter: Signer<'info>,

    pub system_program: Program<'info, System>, // Kept in case needed for future logic
}

/// Accounts required for a player to place bets in the current round.
#[derive(Accounts)]
pub struct PlaceBets<'info> {
    /// The vault corresponding to the token the player is betting with. Mutable to update liquidity and rewards.
    #[account(mut)]
    pub vault: Account<'info, VaultAccount>,

    /// The global `GameSession` account. Mutable to update bet counts.
    #[account(mut, seeds = [b"game_session"], bump = game_session.bump)]
    pub game_session: Account<'info, GameSession>,

    /// CHECK: Validated in instruction logic (is TokenAccount).
    #[account(mut)]
    pub player_token_account: AccountInfo<'info>,

    /// CHECK: Validated in instruction logic (is TokenAccount). Constraint ensures it matches `vault.token_account`.
    #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
    pub vault_token_account: AccountInfo<'info>,

    /// The player placing the bets (signer).
    #[account(mut)]
    pub player: Signer<'info>,

    /// The account storing the player's bets for the current round. MUST exist (initialized via `initialize_player_bets`).
    /// Seeds: [b"player_bets", game_session_key, player_key]
    #[account(
        mut,
        seeds = [b"player_bets", game_session.key().as_ref(), player.key().as_ref()],
        bump = player_bets.bump // Verify bump of existing account
    )]
    pub player_bets: Account<'info, PlayerBets>,

    /// The SPL Token Program, needed for the bet transfer CPI.
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct CloseBets<'info> {
    /// The global `GameSession` account. Mutable to update status and timestamps.
    #[account(mut, seeds = [b"game_session"], bump = game_session.bump)]
    pub game_session: Account<'info, GameSession>,

    /// The user initiating the closing of bets (signer).
    #[account(mut)]
    pub closer: Signer<'info>,

    pub system_program: Program<'info, System>, // Kept in case needed for future logic
}

/// Accounts required to trigger the random number generation for the current round.
#[derive(Accounts)]
pub struct GetRandom<'info> {
    /// The global `GameSession` account. Mutable to store the winning number and update status.
    #[account(mut, seeds = [b"game_session"], bump = game_session.bump)]
    pub game_session: Account<'info, GameSession>,

    /// The user initiating the random generation (signer).
    #[account(mut)]
    pub random_initiator: Signer<'info>,
}

/// Accounts required for a player to claim their winnings for the MOST RECENTLY completed round.
/// Uses the player's LATEST bets recorded in their PlayerBets account.
#[derive(Accounts)]
pub struct ClaimMyWinnings<'info> {
    /// The player claiming the winnings (signer). Pays for `claim_record` creation if needed.
    #[account(mut)]
    pub player: Signer<'info>,

    /// The global game session account. Checked to ensure a winning number exists.
    #[account(
        seeds = [b"game_session"],
        bump = game_session.bump,
        constraint = game_session.winning_number.is_some() @ RouletteError::ClaimRoundMismatchOrNotCompleted,
    )]
    pub game_session: Account<'info, GameSession>, // Needed for winning_number and last_completed_round

    /// The player's bets account, containing their LATEST placed bets, vault, and token mint.
    #[account(
        seeds = [b"player_bets", game_session.key().as_ref(), player.key().as_ref()],
        bump = player_bets.bump,
        constraint = player_bets.player == player.key() @ RouletteError::Unauthorized,
    )]
    pub player_bets: Account<'info, PlayerBets>, // Загружаем аккаунт с последними ставками

    /// The vault corresponding to the LATEST token_mint used by the player in `player_bets`.
    #[account(
        mut,
        seeds = [b"vault", player_bets.token_mint.as_ref()], // Используем mint из последних ставок
        bump = vault.bump,
        constraint = vault.key() == player_bets.vault @ RouletteError::VaultMismatch, // Проверяем vault из последних ставок
    )]
    pub vault: Account<'info, VaultAccount>,

    /// CHECK: Validated manually + via constraint below.
    #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account @ RouletteError::InvalidTokenAccount,
    )]
    pub vault_token_account: AccountInfo<'info>,

    /// CHECK: Validated manually (mint, owner).
    #[account(mut)]
    pub player_token_account: AccountInfo<'info>,

    /// A record created to prevent double-claiming for this player and the completed round.
    #[account(
        init_if_needed,
        payer = player,
        space = 8 + 1 + 1, // Discriminator + claimed: bool + bump: u8
        seeds = [
            b"claim_record",
            player.key().as_ref(),
            game_session.last_completed_round.to_le_bytes().as_ref(), // Используем номер последнего завершенного раунда
        ],
        bump
    )]
    pub claim_record: Account<'info, ClaimRecord>,

    /// SPL Token Program.
    pub token_program: Program<'info, Token>,
    /// System Program (for creating `claim_record`).
    pub system_program: Program<'info, System>,
    /// Rent Sysvar (for creating `claim_record`).
    pub rent: Sysvar<'info, Rent>,
}

/// Divisor for calculating liquidity provider rewards (~1.1%).
const PROVIDER_DIVISOR: u64 = 91;
/// Divisor for calculating program owner revenue (~1.6%).
const OWNER_DIVISOR: u64 = 62;
/// Minimum duration (in seconds) a round must be open for betting.
const MIN_ROUND_DURATION: i64 = 180; // 3 minutes
// TODO: Consider making reward parameters configurable (e.g., in GameSession or separate config account).

/// Constant for 'Straight' bet type.
pub const BET_TYPE_STRAIGHT: u8 = 0;
/// Constant for 'Split' bet type.
pub const BET_TYPE_SPLIT: u8 = 1;
/// Maximum valid numerical value for a bet type enum.
pub const BET_TYPE_MAX: u8 = 15;

#[program]
pub mod roulette_game {
    use super::*;

    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        // PDA check & bump derivation
        let (expected_vault_pda, expected_bump) = Pubkey::find_program_address(
            &[b"vault", ctx.accounts.token_mint.key().as_ref()],
            ctx.program_id
        );
        require_keys_eq!(
            ctx.accounts.vault.key(),
            expected_vault_pda,
            RouletteError::VaultPDAMismatch
        );

        // vault_token_account validation
        let vault_token_info = &ctx.accounts.vault_token_account;
        let vault_token_acc_data = vault_token_info.data.borrow();
        let vault_spl_token_account = TokenAccount::try_deserialize(
            &mut &vault_token_acc_data[..]
        )?;
        require_keys_eq!(
            vault_spl_token_account.mint,
            ctx.accounts.token_mint.key(),
            RouletteError::InvalidTokenAccount
        );
        require_keys_eq!(
            vault_spl_token_account.owner,
            expected_vault_pda,
            RouletteError::InvalidTokenAccountOwner
        );

        // SOL fee transfer
        let transfer_instruction = system_instruction::transfer(
            &ctx.accounts.authority.key(),
            &ctx.accounts.treasury_sol_account.key(),
            CREATE_VAULT_FEE_SOL_LAMPORTS
        );
        invoke(
            &transfer_instruction,
            &[
                ctx.accounts.authority.to_account_info(),
                ctx.accounts.treasury_sol_account.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ]
        )?;

        // Initialize vault state
        let vault = &mut ctx.accounts.vault;
        vault.authority = ctx.accounts.authority.key();
        vault.token_mint = ctx.accounts.token_mint.key();
        vault.token_account = ctx.accounts.vault_token_account.key();
        vault.total_liquidity = 0;
        vault.bump = expected_bump;
        vault.liquidity_pool = Vec::new();
        vault.provider_rewards = Vec::new();
        vault.owner_reward = 0;

        Ok(())
    }

    pub fn provide_liquidity(ctx: Context<ProvideLiquidity>, amount: u64) -> Result<()> {
        // Manual deserialization and validation of token accounts
        let provider_token = TokenAccount::try_deserialize(
            &mut &ctx.accounts.provider_token_account.data.borrow()[..]
        )?;
        let vault_token = TokenAccount::try_deserialize(
            &mut &ctx.accounts.vault_token_account.data.borrow()[..]
        )?;
        let vault = &mut ctx.accounts.vault;
        let provider = &ctx.accounts.liquidity_provider;
        require_eq!(provider_token.mint, vault.token_mint, RouletteError::InvalidTokenAccount);
        require_eq!(vault_token.mint, vault.token_mint, RouletteError::InvalidTokenAccount);

        // Transfer liquidity
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.provider_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: provider.to_account_info(),
            }),
            amount
        )?;

        // Update vault state
        vault.total_liquidity = vault.total_liquidity
            .checked_add(amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        let provider_key = *provider.key;
        // Initialize provider reward entry if not exists
        if !vault.provider_rewards.iter().any(|r| r.provider == provider_key) {
            vault.provider_rewards.push(ProviderReward {
                provider: provider_key,
                accumulated_reward: 0,
            });
        }

        // Record the liquidity provision
        vault.liquidity_pool.push(LiquidityProvision {
            provider: provider_key,
            amount,
            timestamp: Clock::get()?.unix_timestamp,
            withdrawn: false,
        });

        emit!(LiquidityProvided {
            provider: provider_key,
            token_mint: vault.token_mint,
            amount,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    pub fn withdraw_liquidity(ctx: Context<WithdrawLiquidity>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let provider = &ctx.accounts.liquidity_provider;

        // Calculate total available liquidity for the provider
        let mut available_for_withdrawal: u64 = 0;
        for provision in &vault.liquidity_pool {
            if provision.provider == *provider.key && !provision.withdrawn {
                available_for_withdrawal = available_for_withdrawal
                    .checked_add(provision.amount)
                    .ok_or(RouletteError::ArithmeticOverflow)?;
            }
        }

        // Require exact withdrawal amount
        require!(available_for_withdrawal == amount, RouletteError::MustWithdrawExactAmount);
        require!(vault.total_liquidity >= amount, RouletteError::InsufficientLiquidity);

        // Transfer tokens back to provider
        let seeds = &[b"vault".as_ref(), vault.token_mint.as_ref(), &[vault.bump]];
        let signer_seeds = &[&seeds[..]];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.provider_token_account.to_account_info(),
                    authority: vault.to_account_info(),
                },
                signer_seeds
            ),
            amount
        )?;

        // Update vault state
        vault.total_liquidity = vault.total_liquidity
            .checked_sub(amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Mark provisions as withdrawn
        for provision in &mut vault.liquidity_pool {
            if provision.provider == *provider.key && !provision.withdrawn {
                provision.withdrawn = true;
            }
        }

        emit!(LiquidityWithdrawn {
            provider: *provider.key,
            token_mint: vault.token_mint,
            amount,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    pub fn withdraw_provider_revenue(ctx: Context<WithdrawProviderRevenue>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let provider = &ctx.accounts.liquidity_provider;

        // Find provider's reward
        let mut reward_amount: u64 = 0;
        let mut reward_index: Option<usize> = None;
        for (i, reward) in vault.provider_rewards.iter().enumerate() {
            if reward.provider == *provider.key {
                reward_amount = reward.accumulated_reward;
                reward_index = Some(i);
                break;
            }
        }

        require!(reward_amount > 0, RouletteError::NoReward);
        require!(vault.total_liquidity >= reward_amount, RouletteError::InsufficientLiquidity);

        // Transfer reward
        let seeds = &[b"vault".as_ref(), vault.token_mint.as_ref(), &[vault.bump]];
        let signer_seeds = &[&seeds[..]];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.provider_token_account.to_account_info(),
                    authority: vault.to_account_info(),
                },
                signer_seeds
            ),
            reward_amount
        )?;

        // Update vault state
        vault.total_liquidity = vault.total_liquidity
            .checked_sub(reward_amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;
        vault.provider_rewards[reward_index.ok_or(RouletteError::NoReward)?].accumulated_reward = 0;

        Ok(())
    }

    pub fn withdraw_owner_revenue(ctx: Context<WithdrawOwnerRevenue>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        require!(vault.authority == ctx.accounts.authority.key(), RouletteError::Unauthorized);

        let reward_amount = vault.owner_reward;
        require!(reward_amount > 0, RouletteError::NoReward);
        require!(vault.total_liquidity >= reward_amount, RouletteError::InsufficientLiquidity);

        // Transfer reward
        let seeds = &[b"vault".as_ref(), vault.token_mint.as_ref(), &[vault.bump]];
        let signer_seeds = &[&seeds[..]];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.owner_token_account.to_account_info(),
                    authority: vault.to_account_info(),
                },
                signer_seeds
            ),
            reward_amount
        )?;

        // Update vault state
        vault.total_liquidity = vault.total_liquidity
            .checked_sub(reward_amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;
        vault.owner_reward = 0;

        Ok(())
    }

    pub fn initialize_and_provide_liquidity(
        ctx: Context<InitializeAndProvideLiquidity>,
        amount: u64
    ) -> Result<()> {
        // Manual deserialization and validation
        let provider_token_info = &ctx.accounts.provider_token_account;
        let vault_token_info = &ctx.accounts.vault_token_account;
        let _provider_token_account: TokenAccount = TokenAccount::try_deserialize(
            &mut &provider_token_info.data.borrow()[..]
        )?;
        let _vault_token_account: TokenAccount = TokenAccount::try_deserialize(
            &mut &vault_token_info.data.borrow()[..]
        )?;
        let mint_info = &ctx.accounts.token_mint;
        let _mint: Mint = Mint::try_deserialize(&mut &mint_info.data.borrow()[..])?;
        require_eq!(
            _provider_token_account.mint,
            mint_info.key(),
            RouletteError::InvalidTokenAccount
        );
        require_eq!(_vault_token_account.mint, mint_info.key(), RouletteError::InvalidTokenAccount);

        let transfer_instruction = system_instruction::transfer(
            &ctx.accounts.authority.key(),
            &ctx.accounts.treasury_sol_account.key(),
            CREATE_VAULT_FEE_SOL_LAMPORTS
        );
        invoke(
            &transfer_instruction,
            &[
                ctx.accounts.authority.to_account_info(),
                ctx.accounts.treasury_sol_account.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ]
        )?;

        // Initialize vault state
        let vault = &mut ctx.accounts.vault;
        vault.authority = ctx.accounts.authority.key();
        vault.token_mint = ctx.accounts.token_mint.key();
        vault.token_account = ctx.accounts.vault_token_account.key();
        vault.total_liquidity = 0;
        vault.bump = ctx.bumps.vault;
        vault.liquidity_pool = Vec::new();
        vault.provider_rewards = Vec::new();
        vault.owner_reward = 0;
        vault.provider_rewards.push(ProviderReward {
            provider: *ctx.accounts.liquidity_provider.key,
            accumulated_reward: 0,
        });

        // Transfer initial liquidity
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.provider_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.liquidity_provider.to_account_info(),
            }),
            amount
        )?;

        // Update vault state
        vault.total_liquidity = amount;
        vault.liquidity_pool.push(LiquidityProvision {
            provider: *ctx.accounts.liquidity_provider.key,
            amount,
            timestamp: Clock::get()?.unix_timestamp,
            withdrawn: false,
        });

        emit!(LiquidityProvided {
            provider: *ctx.accounts.liquidity_provider.key,
            token_mint: vault.token_mint,
            amount,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    pub fn initialize_game_session(ctx: Context<InitializeGameSession>) -> Result<()> {
        let game_session = &mut ctx.accounts.game_session;
        game_session.current_round = 0;
        game_session.round_start_time = 0;
        game_session.round_status = RoundStatus::NotStarted;
        game_session.winning_number = None;
        game_session.bets_closed_timestamp = 0;
        game_session.get_random_timestamp = 0;
        game_session.bump = ctx.bumps.game_session;
        game_session.last_bettor = None; // Initialize last_bettor
        game_session.last_completed_round = 0;
        Ok(())
    }

    pub fn initialize_player_bets(ctx: Context<InitializePlayerBets>) -> Result<()> {
        msg!("Initializing PlayerBets. Current GameSession status: {:?}", ctx.accounts.game_session.round_status);
        let player_bets = &mut ctx.accounts.player_bets;
        player_bets.player = ctx.accounts.player.key();
        player_bets.round = 0; // Initial round is 0
        player_bets.vault = Pubkey::default(); // Will be set on first bet
        player_bets.token_mint = Pubkey::default(); // Will be set on first bet
        player_bets.bets = Vec::with_capacity(MAX_BETS_PER_ROUND);
        player_bets.bump = ctx.bumps.player_bets;
        msg!("PlayerBets account fields initialized for player {}", ctx.accounts.player.key());
        Ok(())
    }

    /// Closes the player's PlayerBets account for the current game session PDA structure
    /// and returns the rent exemption SOL back to the player.
    /// This should only be called when the player is certain they no longer need
    /// the account (e.g., finished playing or wants to reset).
    pub fn close_player_bets_account(ctx: Context<ClosePlayerBetsAccount>) -> Result<()> {
        // No explicit logic needed here. The `close = player` constraint in the
        // `ClosePlayerBetsAccount` struct handles closing the account and returning the SOL.
        // Anchor automatically transfers the lamports from the `player_bets` account
        // to the `player` account upon successful execution of this instruction.

        msg!(
            "PlayerBets account {} for player {} closed.",
            ctx.accounts.player_bets.key(),
            ctx.accounts.player.key()
        );

        Ok(())
    }

    pub fn start_new_round(ctx: Context<StartNewRound>) -> Result<()> {
        let game_session = &mut ctx.accounts.game_session;
        let current_time = Clock::get()?.unix_timestamp;

        require!(
            game_session.round_status == RoundStatus::NotStarted ||
                game_session.round_status == RoundStatus::Completed,
            RouletteError::RoundInProgress
        );

        game_session.current_round = game_session.current_round
            .checked_add(1)
            .ok_or(RouletteError::ArithmeticOverflow)?;
        game_session.round_start_time = current_time;
        game_session.round_status = RoundStatus::AcceptingBets;
        game_session.winning_number = None;
        game_session.bets_closed_timestamp = 0;
        game_session.get_random_timestamp = 0;
        game_session.last_bettor = None; // Reset last bettor for the new round

        emit!(RoundStarted {
            round: game_session.current_round,
            starter: *ctx.accounts.starter.key,
            start_time: current_time,
        });
        Ok(())
    }

    pub fn place_bet(ctx: Context<PlaceBets>, bet: Bet) -> Result<()> {
        // <<< ПЕРЕИМЕНОВАНО: place_bets -> place_bet
        let game_session = &mut ctx.accounts.game_session;
        let player_bets = &mut ctx.accounts.player_bets;
        let player = &ctx.accounts.player;
        let vault_key = ctx.accounts.vault.key();
        let vault = &mut ctx.accounts.vault;

        require!(
            game_session.round_status == RoundStatus::AcceptingBets,
            RouletteError::BetsNotAccepted
        );
        require!(bet.bet_type <= BET_TYPE_MAX, RouletteError::InvalidBet);

        // Handle first bet in round / round switch
        if player_bets.round != game_session.current_round {
            player_bets.bets.clear(); // Clear previous round's bets
            player_bets.round = game_session.current_round;
            player_bets.vault = vault_key; // Set vault for this round
            player_bets.token_mint = vault.token_mint; // Set mint for this round
            if player_bets.player == Pubkey::default() {
                // Ensure player is set (first ever call)
                player_bets.player = *player.key;
            }
        } else {
            // Subsequent bet, ensure vault hasn't changed
            require_keys_eq!(vault_key, player_bets.vault, RouletteError::VaultMismatch);
        }

        // Check bet vector capacity
        if player_bets.bets.len() >= MAX_BETS_PER_ROUND {
            return err!(RouletteError::InvalidNumberOfBets); // Or MaxBetsInAccountReached
        }

        // Transfer bet amount
        let bet_amount = bet.amount;
        require!(bet_amount > 0, RouletteError::InvalidBet); // Bet amount cannot be zero
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.player_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: player.to_account_info(),
            }),
            bet_amount
        )?;

        // Update vault liquidity
        vault.total_liquidity = vault.total_liquidity
            .checked_add(bet_amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Distribute rewards
        let provider_revenue = bet_amount / PROVIDER_DIVISOR;
        let owner_revenue = bet_amount / OWNER_DIVISOR;
        vault.owner_reward = vault.owner_reward
            .checked_add(owner_revenue)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Calculate provider shares based on active liquidity
        let mut total_active_liquidity: u128 = 0;
        let mut provider_liquidity_map: Vec<(Pubkey, u64)> = Vec::new();
        for provision in &vault.liquidity_pool {
            if !provision.withdrawn {
                let amount_u128 = provision.amount as u128;
                total_active_liquidity = total_active_liquidity
                    .checked_add(amount_u128)
                    .ok_or(RouletteError::ArithmeticOverflow)?;

                if
                    let Some(entry) = provider_liquidity_map
                        .iter_mut()
                        .find(|(key, _)| *key == provision.provider)
                {
                    entry.1 = entry.1
                        .checked_add(provision.amount)
                        .ok_or(RouletteError::ArithmeticOverflow)?;
                } else {
                    provider_liquidity_map.push((provision.provider, provision.amount));
                }
            }
        }

        // Distribute provider revenue proportionally
        if total_active_liquidity > 0 {
            for (provider_key, liquidity_amount) in provider_liquidity_map {
                let provider_share_u128: u128 = (provider_revenue as u128)
                    .checked_mul(liquidity_amount as u128)
                    .ok_or(RouletteError::ArithmeticOverflow)?
                    .checked_div(total_active_liquidity)
                    .ok_or(RouletteError::ArithmeticOverflow)?;

                let provider_share: u64 = provider_share_u128
                    .try_into()
                    .map_err(|_| RouletteError::ArithmeticOverflow)?;

                // Update or initialize provider reward entry
                let mut found = false;
                for reward in &mut vault.provider_rewards {
                    if reward.provider == provider_key {
                        reward.accumulated_reward = reward.accumulated_reward
                            .checked_add(provider_share)
                            .ok_or(RouletteError::ArithmeticOverflow)?;
                        found = true;
                        break;
                    }
                }
                if !found {
                    vault.provider_rewards.push(ProviderReward {
                        provider: provider_key,
                        accumulated_reward: provider_share,
                    });
                }
            }
        }

        // Add bet to player's account
        player_bets.bets.push(bet.clone());

        // Record the last bettor
        game_session.last_bettor = Some(*player.key);

        emit!(BetPlaced {
            player: *player.key,
            token_mint: vault.token_mint,
            round: game_session.current_round,
            bet,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }

    pub fn close_bets(ctx: Context<CloseBets>) -> Result<()> {
        let game_session = &mut ctx.accounts.game_session;
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;

        require!(
            game_session.round_status == RoundStatus::AcceptingBets,
            RouletteError::BetsNotAccepted
        );
        require!(
            current_time >=
                game_session.round_start_time.checked_add(MIN_ROUND_DURATION).unwrap_or(i64::MAX),
            RouletteError::TooEarlyToClose
        );

        game_session.round_status = RoundStatus::BetsClosed;
        game_session.bets_closed_timestamp = current_time;

        emit!(BetsClosed {
            round: game_session.current_round,
            closer: *ctx.accounts.closer.key,
            close_time: current_time,
        });
        Ok(())
    }

    pub fn get_random(ctx: Context<GetRandom>) -> Result<()> {
        let game_session = &mut ctx.accounts.game_session;
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;
        let current_slot = clock.slot;

        require!(
            game_session.round_status == RoundStatus::BetsClosed,
            RouletteError::RandomBeforeClosing
        );
        require!(game_session.winning_number.is_none(), RouletteError::RandomAlreadyGenerated);
        require!(game_session.last_bettor.is_some(), RouletteError::NoBetsPlacedInRound);
        let last_bettor_key = game_session.last_bettor.unwrap();

        // Generate pseudo-random number using SHA256
        let hash_result = hash::hashv(
            &[&last_bettor_key.to_bytes(), &current_time.to_le_bytes(), &current_slot.to_le_bytes()]
        );
        let hash_bytes = hash_result.to_bytes();
        let hash_prefix_u64 = u64::from_le_bytes(hash_bytes[0..8].try_into().unwrap());
        let winning_number = (hash_prefix_u64 % 37) as u8; // Modulo 37 for 0-36

        // Update game session
        game_session.winning_number = Some(winning_number);
        game_session.round_status = RoundStatus::Completed;
        game_session.last_completed_round = game_session.current_round;
        game_session.get_random_timestamp = current_time;

        emit!(RandomGenerated {
            round: game_session.current_round,
            initiator: *ctx.accounts.random_initiator.key,
            winning_number: winning_number,
            generation_time: current_time,
            slot: current_slot,
            last_bettor: last_bettor_key,
        });

        Ok(())
    }
}

impl PlayerBets {
    fn calculate_payout_multiplier(bet_type: u8) -> u64 {
        match bet_type {
            0 => 36, // Straight
            1 => 18, // Split
            2 => 9, // Corner
            3 => 12, // Street
            4 => 6, // SixLine
            5 => 9, // FirstFour
            6 | 7 | 8 | 9 | 10 | 11 => 2, // Red/Black/Even/Odd/Manque/Passe
            12 | 13 | 14 | 15 => 3, // Column/Dozens
            _ => 0, // Unknown
        }
    }

    fn is_bet_winner(bet_type: u8, numbers: &[u8; 4], winning_number: u8) -> bool {
        const RED_NUMBERS: [u8; 18] = [
            1, 3, 5, 7, 9, 12, 14, 16, 18, 19, 21, 23, 25, 27, 30, 32, 34, 36,
        ];

        match bet_type {
            0 => numbers[0] == winning_number, // Straight
            1 => numbers[0] == winning_number || numbers[1] == winning_number, // Split
            2 => {
                // Corner
                let top_left = numbers[0];
                if top_left == 0 || top_left > 34 || top_left % 3 == 0 {
                    return false;
                }
                let corner_numbers = [top_left, top_left + 1, top_left + 3, top_left + 4];
                corner_numbers.contains(&winning_number)
            }
            3 => {
                // Street
                let start_street = numbers[0];
                if
                    start_street == 0 ||
                    start_street > 34 ||
                    (start_street > 0 && (start_street - 1) % 3 != 0)
                {
                    return false;
                }
                winning_number > 0 &&
                    winning_number >= start_street &&
                    winning_number < start_street + 3
            }
            4 => {
                // Six Line
                let start_six_line = numbers[0];
                if
                    start_six_line == 0 ||
                    start_six_line > 31 ||
                    (start_six_line > 0 && (start_six_line - 1) % 3 != 0)
                {
                    return false;
                }
                winning_number > 0 &&
                    winning_number >= start_six_line &&
                    winning_number < start_six_line + 6
            }
            5 => [0, 1, 2, 3].contains(&winning_number), // First Four
            6 => RED_NUMBERS.contains(&winning_number), // Red
            7 => winning_number != 0 && !RED_NUMBERS.contains(&winning_number), // Black
            8 => winning_number != 0 && winning_number % 2 == 0, // Even
            9 => winning_number != 0 && winning_number % 2 == 1, // Odd
            10 => winning_number >= 1 && winning_number <= 18, // Manque (1-18)
            11 => winning_number >= 19 && winning_number <= 36, // Passe (19-36)
            12 => {
                // Column
                let column = numbers[0];
                if column < 1 || column > 3 {
                    return false;
                }
                winning_number != 0 && winning_number % 3 == column % 3
            }
            13 => winning_number >= 1 && winning_number <= 12, // P12 (Dozen 1)
            14 => winning_number >= 13 && winning_number <= 24, // M12 (Dozen 2)
            15 => winning_number >= 25 && winning_number <= 36, // D12 (Dozen 3)
            _ => false, // Unknown
        }
    }
}

pub fn claim_my_winnings(ctx: Context<ClaimMyWinnings>) -> Result<()> {
    // >> ИЗМЕНЕНО: Устанавливаем `round_claimed` из last_completed_round
    let round_claimed = ctx.accounts.game_session.last_completed_round;
    // >> winning_number берется из game_session (он соответствует round_claimed из-за constraint)
    let winning_number = ctx.accounts.game_session.winning_number.unwrap(); // Safe due to constraint

    // >> Используем `player_bets` для получения последних ставок
    let player_bets_account = &ctx.accounts.player_bets;
    // >> Используем `vault`, связанный с последними ставками
    let vault = &mut ctx.accounts.vault;
    let player_token_account_info = &ctx.accounts.player_token_account;
    let vault_token_account_info = &ctx.accounts.vault_token_account;

    // Manual deserialization & validation of token accounts
    let player_token_account: TokenAccount = TokenAccount::try_deserialize(
        &mut &player_token_account_info.data.borrow()[..]
    )?;
    let vault_token_account: TokenAccount = TokenAccount::try_deserialize(
        &mut &vault_token_account_info.data.borrow()[..]
    )?;
    require_keys_eq!(
        vault_token_account_info.key(),
        vault.token_account,
        RouletteError::InvalidTokenAccount
    );
    require_eq!(vault_token_account.mint, vault.token_mint, RouletteError::InvalidTokenAccount);
    require_eq!(player_token_account.mint, vault.token_mint, RouletteError::InvalidTokenAccount);
    require_keys_eq!(
        player_token_account.owner,
        ctx.accounts.player.key(),
        RouletteError::InvalidTokenAccount
    );

    // Calculate total payout from player's bets for the claimed round
    let mut total_payout: u64 = 0;
    for bet in player_bets_account.bets.iter() {
        if PlayerBets::is_bet_winner(bet.bet_type, &bet.numbers, winning_number) {
            let payout_multiplier = PlayerBets::calculate_payout_multiplier(bet.bet_type);
            let payout_for_bet = bet.amount
                .checked_mul(payout_multiplier)
                .ok_or(RouletteError::ArithmeticOverflow)?;
            total_payout = total_payout
                .checked_add(payout_for_bet)
                .ok_or(RouletteError::ArithmeticOverflow)?;
        }
    }

    // Initialize ClaimRecord on first attempt (even if no winnings) to prevent replay
    let claim_record = &mut ctx.accounts.claim_record;
    claim_record.bump = ctx.bumps.claim_record;

    if total_payout == 0 {
        claim_record.claimed = false;
        return err!(RouletteError::NoWinningsFound);
    }

    // Check vault liquidity
    require!(vault.total_liquidity >= total_payout, RouletteError::InsufficientLiquidity);

    // Transfer winnings
    let seeds = &[b"vault".as_ref(), vault.token_mint.as_ref(), &[vault.bump]];
    let signer_seeds = &[&seeds[..]];
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: vault_token_account_info.to_account_info(),
                to: player_token_account_info.to_account_info(),
                authority: vault.to_account_info(),
            },
            signer_seeds
        ),
        total_payout
    )?;

    // Update vault liquidity
    vault.total_liquidity = vault.total_liquidity
        .checked_sub(total_payout)
        .ok_or(RouletteError::ArithmeticOverflow)?;

    // Mark round as claimed
    claim_record.claimed = true;

    emit!(WinningsClaimed {
        round: round_claimed,
        player: ctx.accounts.player.key(),
        token_mint: vault.token_mint,
        amount: total_payout,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

#[account]
pub struct VaultAccount {
    pub authority: Pubkey,
    pub token_mint: Pubkey,
    pub token_account: Pubkey,
    pub total_liquidity: u64,
    pub bump: u8,
    pub liquidity_pool: Vec<LiquidityProvision>,
    pub provider_rewards: Vec<ProviderReward>,
    pub owner_reward: u64,
}

#[account]
#[derive(Default)]
pub struct GameSession {
    pub current_round: u64,
    pub round_start_time: i64,
    pub round_status: RoundStatus,
    pub winning_number: Option<u8>,
    pub bets_closed_timestamp: i64,
    pub get_random_timestamp: i64,
    pub bump: u8,
    pub last_bettor: Option<Pubkey>,
    pub last_completed_round: u64,
}

#[account]
pub struct PlayerBets {
    pub player: Pubkey,
    pub round: u64,
    pub vault: Pubkey,
    pub token_mint: Pubkey,
    pub bets: Vec<Bet>,
    pub bump: u8,
}

/// Record to prevent double-claiming winnings for a specific player and round.
#[account]
#[derive(Default)]
pub struct ClaimRecord {
    /// Whether the winnings for this player/round have been successfully claimed.
    pub claimed: bool,
    /// Bump seed for the PDA.
    pub bump: u8,
}

#[event]
pub struct RoundStarted {
    pub round: u64,
    pub starter: Pubkey,
    pub start_time: i64,
}

#[event]
pub struct WinningsClaimed {
    pub round: u64,
    pub player: Pubkey,
    pub token_mint: Pubkey,
    pub amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct BetsClosed {
    pub round: u64,
    pub closer: Pubkey,
    pub close_time: i64,
}

#[event]
pub struct RandomGenerated {
    pub round: u64,
    pub initiator: Pubkey,
    pub winning_number: u8,
    pub generation_time: i64,
    pub slot: u64,
    pub last_bettor: Pubkey,
}

#[event]
pub struct LiquidityProvided {
    pub provider: Pubkey,
    pub token_mint: Pubkey,
    pub amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct LiquidityWithdrawn {
    pub provider: Pubkey,
    pub token_mint: Pubkey,
    pub amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct BetPlaced {
    pub player: Pubkey,
    pub token_mint: Pubkey,
    pub round: u64,
    pub bet: Bet,
    pub timestamp: i64,
}

#[error_code]
pub enum RouletteError {
    #[msg("Arithmetic overflow error during calculation.")]
    ArithmeticOverflow,
    #[msg("Maximum number of bets per round per player reached.")]
    InvalidNumberOfBets,
    #[msg("Insufficient funds in the player's token account for the bet.")]
    InsufficientFunds,
    #[msg("Insufficient liquidity in the vault to cover payout or withdrawal.")]
    InsufficientLiquidity,
    #[msg("Unauthorized: Signer does not have the required permissions.")]
    Unauthorized,
    #[msg("No reward available for withdrawal (for LPs or owner).")]
    NoReward,
    #[msg("Liquidity withdrawal must match the exact total amount provided and not yet withdrawn.")]
    MustWithdrawExactAmount,
    #[msg("Invalid bet type or numbers provided.")]
    InvalidBet,
    #[msg("Cannot start a new round while one is already in progress.")]
    RoundInProgress,
    #[msg("Bets cannot be placed as the round is not in the 'AcceptingBets' status.")]
    BetsNotAccepted,
    #[msg("The current round status does not allow this operation.")]
    InvalidRoundStatus,
    #[msg("Too early to close bets; the minimum round duration has not elapsed.")]
    TooEarlyToClose,
    #[msg("Too early for payouts; necessary processing or delay period not complete.")]
    TooEarlyForPayouts,
    #[msg("Player has no bets recorded for this round.")]
    NoBetsInRound,
    #[msg("The global GameSession account was not found or is not initialized.")]
    GameSessionNotFound,
    #[msg("The provided reward token mint or account does not match the configured reward mint.")]
    InvalidRewardToken,
    #[msg("The vault specified does not match the vault associated with the PlayerBets account or expected PDA.")]
    VaultMismatch,
    #[msg("Cannot generate the random number before the betting phase is closed.")]
    RandomBeforeClosing,
    #[msg("The random number for this round has already been generated.")]
    RandomAlreadyGenerated,
    #[msg("The provided PlayerBets account is invalid or does not match expectations.")]
    InvalidPlayerBetsAccount,
    #[msg("Game session account is already initialized.")]
    AlreadyInitialized,
    #[msg("Cannot generate random number because no bets were placed in this round.")]
    NoBetsPlacedInRound,
    #[msg("The vault's token account is not owned by the vault PDA.")]
    InvalidTokenAccountOwner,
    #[msg("Derived vault PDA does not match the provided account.")]
    VaultPDAMismatch,
    #[msg("Invalid SPL token account provided (e.g., wrong mint, owner, or not initialized).")]
    InvalidTokenAccount,
    #[msg("Attempting to claim winnings for a round where the winning number is not available.")]
    ClaimRoundMismatchOrNotCompleted, // Used for winning_number.is_some() check
    #[msg("No winnings found for the player in the specified round (claim attempted).")]
    NoWinningsFound,
}
