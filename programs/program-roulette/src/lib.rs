use anchor_lang::prelude::*;
use anchor_spl::token::{ self, Token, Transfer, Mint, TokenAccount };
use sha2::{ Sha256, Digest };

declare_id!("GZB6nqB9xSC8VKwWajtCu2TotPXz1mZCR5VwMLEKDj81");

const MAX_BETS_PER_TX: usize = 3; // Максимум 3 ставки в транзакции
const PROVIDER_DIVISOR: u64 = 91; // 1/91 ≈ 0.011 (примерно 1.1%)
const OWNER_DIVISOR: u64 = 62; // 1/62 ≈ 0.016 (примерно 1.6%)
const STARTER_REWARD_AMOUNT: u64 = 10_000_000; // 10 токен (с 6 десятичными знаками)
const CLOSER_REWARD_AMOUNT: u64 = 10_000_000; // 10 токена (с 6 десятичными знаками)
const MIN_ROUND_DURATION: i64 = 180; // 3 минуты в секундах
pub const REWARD_TOKEN_MINT: Pubkey = solana_program::pubkey!(
    "4QhhRXq8NvyjpxLiC44nEFzKX9ZRYr9twMvBr7Jm7cLb"
);
const RANDOM_INITIATOR_REWARD: u64 = 15_000_000; // 15 токена (с 6 десятичными знаками)

pub const BET_TYPE_STRAIGHT: u8 = 0;
pub const BET_TYPE_SPLIT: u8 = 1;
pub const BET_TYPE_MAX: u8 = 15; // Максимальный индекс типа ставки

#[program]
pub mod roulette_game {
    use super::*;

    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        let _ = Mint::try_deserialize(&mut &ctx.accounts.token_mint.data.borrow()[..])?;
        let token_account = TokenAccount::try_deserialize(
            &mut &ctx.accounts.vault_token_account.data.borrow()[..]
        )?;
        require!(
            token_account.mint == ctx.accounts.token_mint.key(),
            RouletteError::InvalidTokenAccount
        );
        let vault = &mut ctx.accounts.vault;
        vault.authority = ctx.accounts.authority.key();
        vault.token_mint = ctx.accounts.token_mint.key();
        vault.token_account = ctx.accounts.vault_token_account.key();
        vault.total_liquidity = 0;
        vault.bump = ctx.bumps.vault;
        vault.liquidity_pool = Vec::new();
        vault.total_turnover = 0;
        vault.provider_rewards = Vec::new();
        vault.owner_reward = 0;

        msg!("Vault initialized for token {}", ctx.accounts.token_mint.key());
        Ok(())
    }

    pub fn provide_liquidity(ctx: Context<ProvideLiquidity>, amount: u64) -> Result<()> {
        // Проверяем, что аккаунты действительно являются токен-аккаунтами
        let _provider_token = TokenAccount::try_deserialize(
            &mut &ctx.accounts.provider_token_account.data.borrow()[..]
        )?;
        let _vault_token = TokenAccount::try_deserialize(
            &mut &ctx.accounts.vault_token_account.data.borrow()[..]
        )?;

        let vault = &mut ctx.accounts.vault;
        let provider = &ctx.accounts.liquidity_provider;

        // Перевод токенов от провайдера ликвидности в Vault
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.provider_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: provider.to_account_info(),
            }),
            amount
        )?;

        // Обновляем общую ликвидность
        vault.total_liquidity = vault.total_liquidity
            .checked_add(amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Проверяем, существует ли уже запись для данного провайдера в rewards
        let mut provider_found = false;
        for reward in &mut vault.provider_rewards {
            if reward.provider == *provider.key {
                provider_found = true;
                break;
            }
        }

        // Если провайдера нет в списке вознаграждений, добавляем
        if !provider_found {
            vault.provider_rewards.push(ProviderReward {
                provider: *provider.key,
                accumulated_reward: 0,
            });
        }

        // Добавляем запись о внесенной ликвидности
        vault.liquidity_pool.push(LiquidityProvision {
            provider: *provider.key,
            amount,
            timestamp: Clock::get()?.unix_timestamp,
            withdrawn: false,
        });

        emit!(LiquidityProvided {
            provider: *provider.key,
            token_mint: vault.token_mint,
            amount,
            timestamp: Clock::get()?.unix_timestamp,
        });

        msg!("Liquidity provided: {} tokens", amount);
        Ok(())
    }

    pub fn withdraw_liquidity(ctx: Context<WithdrawLiquidity>, amount: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let provider = &ctx.accounts.liquidity_provider;

        // Находим все депозиты пользователя и считаем доступную сумму
        let mut available_for_withdrawal: u64 = 0;

        for provision in &vault.liquidity_pool {
            if provision.provider == *provider.key && !provision.withdrawn {
                available_for_withdrawal = available_for_withdrawal
                    .checked_add(provision.amount)
                    .ok_or(RouletteError::ArithmeticOverflow)?;
            }
        }

        // Проверяем, что у провайдера достаточно средств для вывода
        require!(available_for_withdrawal == amount, RouletteError::MustWithdrawExactAmount);

        // Проверяем, что в хранилище достаточно ликвидности
        require!(vault.total_liquidity >= amount, RouletteError::InsufficientLiquidity);

        // Создаем семена для подписи Vault PDA
        let seeds = [b"vault".as_ref(), vault.token_mint.as_ref(), &[vault.bump]];
        let signer_seeds = &[&seeds[..]];

        // Перевод токенов от Vault провайдеру (только ликвидность)
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

        // Обновляем общую ликвидность
        vault.total_liquidity = vault.total_liquidity
            .checked_sub(amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Отмечаем все депозиты как выведенные
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

        msg!("Liquidity withdrawn: {}", amount);
        Ok(())
    }

    pub fn withdraw_provider_revenue(ctx: Context<WithdrawProviderRevenue>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let provider = &ctx.accounts.liquidity_provider;

        // Находим накопленное вознаграждение провайдера
        let mut reward_amount: u64 = 0;
        let mut reward_index: usize = 0;
        let mut found = false;

        for (i, reward) in vault.provider_rewards.iter().enumerate() {
            if reward.provider == *provider.key {
                reward_amount = reward.accumulated_reward;
                reward_index = i;
                found = true;
                break;
            }
        }

        // Проверяем, что у провайдера есть накопленное вознаграждение
        require!(found, RouletteError::NoReward);
        require!(reward_amount > 0, RouletteError::NoReward);

        // Проверяем, что в хранилище достаточно ликвидности
        require!(vault.total_liquidity >= reward_amount, RouletteError::InsufficientLiquidity);

        // Создаем семена для подписи Vault PDA
        let seeds = [b"vault".as_ref(), vault.token_mint.as_ref(), &[vault.bump]];
        let signer_seeds = &[&seeds[..]];

        // Перевод вознаграждения от Vault провайдеру
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

        // Обновляем общую ликвидность
        vault.total_liquidity = vault.total_liquidity
            .checked_sub(reward_amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Обнуляем накопленное вознаграждение провайдера
        vault.provider_rewards[reward_index].accumulated_reward = 0;

        emit!(ProviderRevenueWithdrawn {
            provider: *provider.key,
            token_mint: vault.token_mint,
            amount: reward_amount,
            timestamp: Clock::get()?.unix_timestamp,
        });

        msg!("Provider reward withdrawn: {}", reward_amount);
        Ok(())
    }

    pub fn withdraw_owner_revenue(ctx: Context<WithdrawOwnerRevenue>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        // Проверяем, что запрос от владельца системы
        require!(vault.authority == ctx.accounts.authority.key(), RouletteError::Unauthorized);

        // Проверяем, что есть накопленное вознаграждение
        let reward_amount = vault.owner_reward;
        require!(reward_amount > 0, RouletteError::NoReward);

        // Проверяем, что в хранилище достаточно ликвидности
        require!(vault.total_liquidity >= reward_amount, RouletteError::InsufficientLiquidity);

        // Создаем семена для подписи Vault PDA
        let seeds = [b"vault".as_ref(), vault.token_mint.as_ref(), &[vault.bump]];
        let signer_seeds = &[&seeds[..]];

        // Перевод вознаграждения от Vault владельцу
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

        // Обновляем общую ликвидность
        vault.total_liquidity = vault.total_liquidity
            .checked_sub(reward_amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Обнуляем накопленное вознаграждение владельца
        vault.owner_reward = 0;

        emit!(OwnerRevenueWithdrawn {
            owner: vault.authority,
            token_mint: vault.token_mint,
            amount: reward_amount,
            timestamp: Clock::get()?.unix_timestamp,
        });

        msg!("Owner reward withdrawn: {}", reward_amount);
        Ok(())
    }

    // Функция для автоматической инициализации хранилища при предоставлении ликвидности
    // если хранилище для данного токена еще не существует
    pub fn initialize_and_provide_liquidity(
        ctx: Context<InitializeAndProvideLiquidity>,
        amount: u64
    ) -> Result<()> {
        // Инициализируем хранилище
        let vault = &mut ctx.accounts.vault;
        vault.authority = ctx.accounts.authority.key();
        vault.token_mint = ctx.accounts.token_mint.key();
        vault.token_account = ctx.accounts.vault_token_account.key();
        vault.total_liquidity = 0;
        vault.bump = ctx.bumps.vault;
        vault.liquidity_pool = Vec::new();
        vault.total_turnover = 0;
        vault.provider_rewards = Vec::new();
        vault.owner_reward = 0;

        // Добавляем провайдера в список вознаграждений
        vault.provider_rewards.push(ProviderReward {
            provider: *ctx.accounts.liquidity_provider.key,
            accumulated_reward: 0,
        });

        msg!("Vault инициализирован для токена {}", ctx.accounts.token_mint.key());

        // Переводим токены в хранилище
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.provider_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.liquidity_provider.to_account_info(),
            }),
            amount
        )?;

        // Обновляем общую ликвидность
        vault.total_liquidity = amount;

        // Добавляем запись о внесенной ликвидности
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

        msg!("Liquidity provided: {} tokens", amount);
        Ok(())
    }

    pub fn initialize_game_session(ctx: Context<InitializeGameSession>) -> Result<()> {
        let game_session = &mut ctx.accounts.game_session;

        game_session.current_round = 0;
        game_session.round_start_time = 0;
        game_session.round_status = RoundStatus::NotStarted;
        game_session.winning_number = None;
        game_session.starter = None;
        game_session.closer = None;
        game_session.bets_count = 0;
        game_session.total_bet_amount = 0;
        game_session.vaults = Vec::new();
        // Задаем токен для вознаграждений
        game_session.reward_token_mint = ctx.accounts.reward_token_mint.key();
        game_session.bump = ctx.bumps.game_session;
        // Инициализируем round_bets как пустой вектор
        game_session.round_bets = Vec::new();

        msg!("Game session initialized with reward token {}", ctx.accounts.reward_token_mint.key());
        Ok(())
    }
    // Обновляем функцию инициализации игровой сессии
    // Обновляем функцию старта нового раунда
    pub fn start_new_round(ctx: Context<StartNewRound>) -> Result<()> {
        let game_session = &mut ctx.accounts.game_session;
        let current_time = Clock::get()?.unix_timestamp;

        // Проверяем, что предыдущий раунд завершен или это первый раунд
        require!(
            game_session.round_status == RoundStatus::NotStarted ||
                game_session.round_status == RoundStatus::Completed,
            RouletteError::RoundInProgress
        );

        // Check that reward token matches the configured one
        require!(
            ctx.accounts.reward_vault.token_mint == game_session.reward_token_mint,
            RouletteError::InvalidRewardToken
        );

        // Increment round counter
        game_session.current_round = game_session.current_round
            .checked_add(1)
            .ok_or(RouletteError::ArithmeticOverflow)?;
        game_session.round_start_time = current_time;
        game_session.round_status = RoundStatus::AcceptingBets;
        game_session.winning_number = None;
        game_session.starter = Some(*ctx.accounts.starter.key);
        game_session.closer = None;
        game_session.bets_count = 0;
        game_session.total_bet_amount = 0;

        // Give reward to the starter
        let reward_vault = &mut ctx.accounts.reward_vault;
        let seeds = [b"vault".as_ref(), reward_vault.token_mint.as_ref(), &[reward_vault.bump]];
        let signer_seeds = &[&seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.reward_vault_token_account.to_account_info(),
                    to: ctx.accounts.starter_token_account.to_account_info(),
                    authority: reward_vault.to_account_info(),
                },
                signer_seeds
            ),
            STARTER_REWARD_AMOUNT
        )?;

        // Update vault liquidity
        reward_vault.total_liquidity = reward_vault.total_liquidity
            .checked_sub(STARTER_REWARD_AMOUNT)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        emit!(RoundStarted {
            round: game_session.current_round,
            starter: *ctx.accounts.starter.key,
            reward_token_mint: reward_vault.token_mint,
            start_time: current_time,
        });

        msg!(
            "Round {} started by {} with reward token {}",
            game_session.current_round,
            ctx.accounts.starter.key,
            reward_vault.token_mint
        );
        Ok(())
    }

    // Обновляем функцию place_bets
    pub fn place_bets(ctx: Context<PlaceBets>, bets: Vec<Bet>) -> Result<()> {
        require!(
            bets.len() > 0 && bets.len() <= MAX_BETS_PER_TX,
            RouletteError::InvalidNumberOfBets
        );
        let player = &ctx.accounts.player;
        let vault = &mut ctx.accounts.vault;
        let game_session = &mut ctx.accounts.game_session;

        // Check that current round is accepting bets
        require!(
            game_session.round_status == RoundStatus::AcceptingBets,
            RouletteError::BetsNotAccepted
        );

        // Validate bets
        for bet in &bets {
            // Проверка общего диапазона (все числа должны быть в диапазоне 0-36)
            for num in &bet.numbers {
                if *num > 36 && *num != 0 {
                    // 0 можно использовать как "нет значения"
                    return Err(RouletteError::InvalidBet.into());
                }
            }

            // Проверка, что тип ставки существует
            if bet.bet_type > BET_TYPE_MAX {
                return Err(RouletteError::InvalidBet.into());
            }
        }

        // Calculate total bet amount
        let mut total_bet_amount: u64 = 0;
        for bet in &bets {
            total_bet_amount = total_bet_amount
                .checked_add(bet.amount)
                .ok_or(RouletteError::ArithmeticOverflow)?;
        }

        // Transfer tokens from player to Vault
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), Transfer {
                from: ctx.accounts.player_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.player.to_account_info(),
            }),
            total_bet_amount
        )?;

        // Update total liquidity
        vault.total_liquidity = vault.total_liquidity
            .checked_add(total_bet_amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Update total turnover
        vault.total_turnover = vault.total_turnover
            .checked_add(total_bet_amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Update session bets count and amount
        game_session.bets_count = game_session.bets_count
            .checked_add(bets.len() as u64)
            .ok_or(RouletteError::ArithmeticOverflow)?;
        game_session.total_bet_amount = game_session.total_bet_amount
            .checked_add(total_bet_amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Calculate revenue for liquidity providers (1.1% of bet amount)
        let provider_revenue = total_bet_amount / PROVIDER_DIVISOR;

        // Calculate revenue for system owner (1.6% of bet amount)
        let owner_revenue = total_bet_amount / OWNER_DIVISOR;

        // Update owner reward
        vault.owner_reward = vault.owner_reward
            .checked_add(owner_revenue)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Distribute revenue among providers proportionally to their liquidity
        let mut total_active_liquidity: u64 = 0;
        let mut provider_liquidity_map: Vec<(Pubkey, u64)> = Vec::new();

        // First calculate active liquidity of each provider
        for provision in &vault.liquidity_pool {
            if !provision.withdrawn {
                let mut provider_found = false;
                for (provider, amount) in &mut provider_liquidity_map {
                    if *provider == provision.provider {
                        *amount = amount
                            .checked_add(provision.amount)
                            .ok_or(RouletteError::ArithmeticOverflow)?;
                        provider_found = true;
                        break;
                    }
                }

                if !provider_found {
                    provider_liquidity_map.push((provision.provider, provision.amount));
                }

                total_active_liquidity = total_active_liquidity
                    .checked_add(provision.amount)
                    .ok_or(RouletteError::ArithmeticOverflow)?;
            }
        }

 // Then distribute revenue proportionally to contribution
 if total_active_liquidity > 0 {
    for (provider_key, liquidity_amount) in provider_liquidity_map {
        // --- НАЧАЛО ИЗМЕНЕНИЙ: Используем u128 для расчета доли ---
        let provider_share_u128: u128 = (provider_revenue as u128) // Кастуем к u128
            .checked_mul(liquidity_amount as u128) // Умножение в u128
            .ok_or(RouletteError::ArithmeticOverflow)?
            .checked_div(total_active_liquidity as u128) // Деление в u128
            .ok_or(RouletteError::ArithmeticOverflow)?; // Деление на 0 не должно произойти из-за if total_active_liquidity > 0

        // Пытаемся конвертировать результат обратно в u64
        let provider_share: u64 = provider_share_u128
            .try_into() // Безопасное преобразование u128 -> u64
            .map_err(|_| RouletteError::ArithmeticOverflow)?; // Ошибка, если результат все еще слишком велик для u64
        // --- КОНЕЦ ИЗМЕНЕНИЙ ---

                // Update provider's accumulated reward
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

                // If no reward record exists, create a new one
                if !found {
                    vault.provider_rewards.push(ProviderReward {
                        provider: provider_key,
                        accumulated_reward: provider_share,
                    });
                }
            }
        }

        // Save player's bets
        let player_bets = &mut ctx.accounts.player_bets;

        // Initialize player_bets if it's new
        if player_bets.player == Pubkey::default() {
            player_bets.player = *player.key;
            player_bets.round = game_session.current_round;
            player_bets.vault = vault.key();
            player_bets.token_mint = vault.token_mint;
            player_bets.processed = false;
            player_bets.bump = ctx.bumps.player_bets;
        }

        // Add current bets to existing ones
        for bet in bets.iter() {
            player_bets.bets.push(bet.clone());

            let mut vault_found = false;
            for (vault_key, player_bets_list) in &mut game_session.round_bets {
                if *vault_key == vault.key() {
                    // Если хранилище уже в списке, добавляем ставки этого игрока
                    if !player_bets_list.contains(&player_bets.key()) {
                        player_bets_list.push(player_bets.key());
                    }
                    vault_found = true;
                    break;
                }
            }

            // Если хранилище не найдено, добавляем новую запись
            if !vault_found {
                game_session.round_bets.push((vault.key(), vec![player_bets.key()]));
            }
        }

        emit!(BetsPlaced {
            player: *player.key,
            token_mint: vault.token_mint,
            round: game_session.current_round,
            bets: bets.clone(),
            total_amount: total_bet_amount,
            timestamp: Clock::get()?.unix_timestamp,
        });

        msg!(
            "Player {} placed {} bets in round {} for a total of {} tokens of mint {}",
            player.key,
            bets.len(),
            game_session.current_round,
            total_bet_amount,
            vault.token_mint
        );
        Ok(())
    }

    // Обновляем функцию close_bets
    pub fn close_bets(ctx: Context<CloseBets>) -> Result<()> {
        let game_session = &mut ctx.accounts.game_session;
        let current_time = Clock::get()?.unix_timestamp;

        // Проверяем, что текущий раунд принимает ставки
        require!(
            game_session.round_status == RoundStatus::AcceptingBets,
            RouletteError::InvalidRoundStatus
        );

        // Проверяем, что прошло минимальное время
        require!(
            current_time >= game_session.round_start_time + MIN_ROUND_DURATION,
            RouletteError::TooEarlyToClose
        );

        // Проверяем соответствие токена вознаграждения
        require!(
            ctx.accounts.reward_vault.token_mint == game_session.reward_token_mint,
            RouletteError::InvalidRewardToken
        );

        // Закрываем период приема ставок
        game_session.round_status = RoundStatus::BetsClosed;
        game_session.closer = Some(*ctx.accounts.closer.key);

        // Выдаем вознаграждение закрывающему
        let reward_vault = &mut ctx.accounts.reward_vault;
        let seeds = [b"vault".as_ref(), reward_vault.token_mint.as_ref(), &[reward_vault.bump]];
        let signer_seeds = &[&seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.reward_vault_token_account.to_account_info(),
                    to: ctx.accounts.closer_token_account.to_account_info(),
                    authority: reward_vault.to_account_info(),
                },
                signer_seeds
            ),
            CLOSER_REWARD_AMOUNT
        )?;

        // Обновляем ликвидность хранилища
        reward_vault.total_liquidity = reward_vault.total_liquidity
            .checked_sub(CLOSER_REWARD_AMOUNT)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        emit!(BetsClosed {
            round: game_session.current_round,
            closer: *ctx.accounts.closer.key,
            close_time: current_time,
        });

        msg!(
            "Round {} bets closed by {}. Waiting for random number generation.",
            game_session.current_round,
            ctx.accounts.closer.key
        );

        Ok(())
    }

    pub fn get_random(ctx: Context<GetRandom>) -> Result<()> {
        let game_session = &mut ctx.accounts.game_session;
        let current_time = Clock::get()?.unix_timestamp;

        // Проверяем, что ставки закрыты
        require!(
            game_session.round_status == RoundStatus::BetsClosed,
            RouletteError::InvalidRoundStatus
        );

        // Генерируем случайное число
        let clock = Clock::get()?;
        let slot = clock.slot.to_le_bytes();
        let round = game_session.current_round.to_le_bytes();
        let timestamp = clock.unix_timestamp.to_le_bytes();
        let random_initiator = ctx.accounts.random_initiator.key().to_bytes();

        let mut hasher = Sha256::new();
        hasher.update(&slot);
        hasher.update(&round);
        hasher.update(&timestamp);
        hasher.update(&random_initiator);
        let result = hasher.finalize();

        // Конвертируем первые 8 байт в u64 и получаем число от 0 до 36
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&result[0..8]);
        let random_number = u64::from_le_bytes(bytes) % 37;
        let winning_number = random_number as u8;

        // Сохраняем выигрышное число
        game_session.winning_number = Some(winning_number);

        // Выдаем вознаграждение инициатору рандома
        let reward_vault = &mut ctx.accounts.reward_vault;
        let seeds = [b"vault".as_ref(), reward_vault.token_mint.as_ref(), &[reward_vault.bump]];
        let signer_seeds = &[&seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.reward_vault_token_account.to_account_info(),
                    to: ctx.accounts.initiator_token_account.to_account_info(),
                    authority: reward_vault.to_account_info(),
                },
                signer_seeds
            ),
            RANDOM_INITIATOR_REWARD
        )?;

        // Обновляем ликвидность хранилища вознаграждений
        reward_vault.total_liquidity = reward_vault.total_liquidity
            .checked_sub(RANDOM_INITIATOR_REWARD)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        // Переводим раунд в статус "Завершен" - выплаты теперь доступны через claim_winnings
        game_session.round_status = RoundStatus::Completed;

        // Эмитируем события
        emit!(RandomGenerated {
            round: game_session.current_round,
            initiator: *ctx.accounts.random_initiator.key,
            winning_number,
            generation_time: current_time,
        });

        emit!(RoundCompleted {
            round: game_session.current_round,
            completer: *ctx.accounts.random_initiator.key,
            timestamp: current_time,
        });

        // Логируем детали для проверяемости
        msg!(
            "Round {} random generated by {}. Winning number: {}. Random number generation details: slot={}, round={}, timestamp={}, hash={}",
            game_session.current_round,
            ctx.accounts.random_initiator.key,
            winning_number,
            clock.slot,
            game_session.current_round,
            clock.unix_timestamp,
            hex::encode(&result[0..8])
        );

        msg!(
            "Round {} completed. Players can now claim their winnings.",
            game_session.current_round
        );

        Ok(())
    }
    //выплата по запросу игрока
    pub fn claim_winnings(ctx: Context<ClaimWinnings>) -> Result<()> {
        let game_session = &ctx.accounts.game_session;
        let player_bets = &mut ctx.accounts.player_bets;
        let vault = &mut ctx.accounts.vault;
        let player_global_winnings = &mut ctx.accounts.player_global_winnings;

        // Проверяем, что у раунда есть выигрышное число
        let winning_number = game_session.winning_number.ok_or(RouletteError::WinningNumberNotSet)?;

        // Определяем сумму для выплаты
        let mut payout: u64 = 0;

        // Если есть оставшаяся невыплаченная сумма, берем её
        if player_bets.remaining_payout > 0 {
            payout = player_bets.remaining_payout;
        } else if !player_bets.processed {
            // Вычисляем выигрыш
            for bet in &player_bets.bets {
                if PlayerBets::is_bet_winner(bet.bet_type, &bet.numbers, winning_number) {
                    let bet_payout = match
                        bet.amount.checked_mul(
                            PlayerBets::calculate_payout_multiplier(bet.bet_type)
                        )
                    {
                        Some(value) => value,
                        None => {
                            msg!(
                                "Предупреждение: переполнение при расчете выигрыша, используется максимальное значение"
                            );
                            u64::MAX
                        }
                    };

                    // Безопасное сложение с обработкой ошибок
                    payout = match payout.checked_add(bet_payout) {
                        Some(value) => value,
                        None => {
                            msg!(
                                "Предупреждение: переполнение при суммировании выигрышей, используется максимальное значение"
                            );
                            u64::MAX
                        }
                    };
                }
            }

            // Отмечаем ставки как обработанные
            player_bets.processed = true;
        } else {
            // Ставки уже обработаны и нет оставшейся суммы для выплаты
            return Err(RouletteError::PayoutAlreadyProcessed.into());
        }

        // Если есть выигрыш, добавляем его в глобальный аккаунт выигрышей
        if payout > 0 {
            // Добавляем в накопленные выигрыши игрока
            player_global_winnings.accumulated_winnings =
                player_global_winnings.accumulated_winnings
                    .checked_add(payout)
                    .ok_or(RouletteError::ArithmeticOverflow)?;

            // Обнуляем оставшуюся сумму, так как теперь она в глобальном аккаунте
            player_bets.remaining_payout = 0;

            // Эмитируем событие
            emit!(PayoutClaimed {
                round: player_bets.round,
                player: player_bets.player,
                token_mint: player_bets.token_mint,
                amount: payout,
                timestamp: Clock::get()?.unix_timestamp,
            });

            msg!("Добавлено {} токенов в выигрыши игрока {}", payout, player_bets.player);
        } else {
            msg!(
                "Игрок {} не имеет выигрышных ставок в раунде {}",
                player_bets.player,
                player_bets.round
            );
        }

        Ok(())
    }

    pub fn withdraw_all_winnings(ctx: Context<WithdrawAllWinnings>) -> Result<()> {
        let player_winnings = &mut ctx.accounts.player_global_winnings;
        let vault = &mut ctx.accounts.vault;

        // Проверяем, что есть выигрыш для вывода
        let total_amount = player_winnings.accumulated_winnings;
        require!(total_amount > 0, RouletteError::NoReward);

        // Выплачиваем всю сумму
        let seeds = [b"vault".as_ref(), vault.token_mint.as_ref(), &[vault.bump]];
        let signer_seeds = &[&seeds[..]];

        let actual_amount = if total_amount > vault.total_liquidity {
            vault.total_liquidity // Выплачиваем максимально возможную сумму
        } else {
            total_amount
        };

        // Перевод токенов
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.player_token_account.to_account_info(),
                    authority: vault.to_account_info(),
                },
                signer_seeds
            ),
            actual_amount
        )?;

        // Обновляем состояние
        vault.total_liquidity = vault.total_liquidity
            .checked_sub(actual_amount)
            .ok_or(RouletteError::ArithmeticOverflow)?;

        player_winnings.accumulated_winnings = 0;
        player_winnings.last_update = Clock::get()?.unix_timestamp;

        emit!(TotalWinningsWithdrawn {
            player: player_winnings.player,
            token_mint: player_winnings.token_mint,
            amount: actual_amount,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    impl PlayerBets {
        fn calculate_payout_multiplier(bet_type: u8) -> u64 {
            match bet_type {
                0 => 36,
                1 => 18,
                2 => 9,
                3 => 12,
                4 => 6,
                5 => 9,
                6 => 2,
                7 => 2,
                8 => 2,
                9 => 2,
                10 => 2,
                11 => 2,
                12 => 3,
                13 => 3,
                14 => 3,
                15 => 3,
                _ => 0,
            }
        }

        fn is_bet_winner(bet_type: u8, numbers: &[u8; 4], winning_number: u8) -> bool {
            match bet_type {
                0 => numbers[0] == winning_number,

                1 => numbers[0] == winning_number || numbers[1] == winning_number,

                2 => {
                    // Вычисляем 4 числа на основе верхнего левого угла
                    let top_left = numbers[0];
                    let corner_numbers = [top_left, top_left + 1, top_left + 3, top_left + 4];
                    corner_numbers.contains(&winning_number)
                }

                3 => {
                    let street = numbers[0]; // Номер улицы (1-12)
                    let start = (street - 1) * 3 + 1;
                    winning_number >= start && winning_number < start + 3
                }

                4 => {
                    let six_line = numbers[0]; // Номер линии (1-11)
                    let start = (six_line - 1) * 3 + 1;
                    winning_number >= start && winning_number < start + 6
                }

                5 => winning_number <= 3,

                6 =>
                    [1, 3, 5, 7, 9, 12, 14, 16, 18, 19, 21, 23, 25, 27, 30, 32, 34, 36].contains(
                        &winning_number
                    ),

                7 =>
                    [2, 4, 6, 8, 10, 11, 13, 15, 17, 20, 22, 24, 26, 28, 29, 31, 33, 35].contains(
                        &winning_number
                    ),

                8 => winning_number > 0 && winning_number % 2 == 0,

                9 => winning_number > 0 && winning_number % 2 == 1,

                10 => winning_number > 0 && winning_number <= 18,

                11 => winning_number >= 19 && winning_number <= 36,

                12 => {
                    let column = numbers[0]; // Номер колонки (1-3)
                    winning_number > 0 && winning_number % 3 == column % 3
                }

                13 => winning_number >= 1 && winning_number <= 12,

                14 => winning_number >= 13 && winning_number <= 24,

                15 => winning_number >= 25 && winning_number <= 36,

                // Неизвестный тип ставки
                _ => false,
            }
        }
    }
    #[account]
    pub struct VaultAccount {
        pub authority: Pubkey, // Владелец системы
        pub token_mint: Pubkey, // Mint-адрес токена
        pub token_account: Pubkey, // Токен-аккаунт для хранения токенов
        pub total_liquidity: u64, // Общая ликвидность
        pub bump: u8, // Bump для PDA
        pub liquidity_pool: Vec<LiquidityProvision>, // Пул ликвидности
        pub total_turnover: u64, // Общий оборот в данном токене
        pub provider_rewards: Vec<ProviderReward>, // Вознаграждения провайдеров ликвидности
        pub owner_reward: u64, // Вознаграждение владельца системы
    }

    #[account]
    pub struct GameSession {
        pub current_round: u64, // Номер текущего раунда
        pub round_start_time: i64, // Время начала текущего раунда
        pub round_status: RoundStatus, // Статус текущего раунда
        pub winning_number: Option<u8>, // Выигрышное число текущего раунда
        pub starter: Option<Pubkey>, // Игрок, начавший текущий раунд
        pub closer: Option<Pubkey>, // Игрок, закрывший ставки в текущем раунде
        pub bets_count: u64, // Количество ставок в текущем раунде
        pub total_bet_amount: u64, // Общая сумма ставок в текущем раунде
        pub vaults: Vec<Pubkey>, // Список активных хранилищ токенов
        pub reward_token_mint: Pubkey, // Mint-адрес токена для вознаграждений
        pub bump: u8, // Bump для PDA
        // Нужно добавить эту новую структуру для отслеживания всех ставок
        pub round_bets: Vec<(Pubkey, Vec<Pubkey>)>, // Список ставок в формате (vault_pubkey, [player_bets_pubkeys])
    }

    #[account]
    pub struct PlayerBets {
        pub player: Pubkey, // Игрок, сделавший ставки
        pub round: u64, // Номер раунда
        pub vault: Pubkey, // Выбранное хранилище токенов
        pub token_mint: Pubkey, // Mint-адрес токена ставки
        pub bets: Vec<Bet>, // Список ставок
        pub processed: bool, // Флаг обработки выплаты
        pub bump: u8, // Bump для PDA
        pub remaining_payout: u64,
    }

    #[account]
    pub struct PlayerGlobalWinnings {
        pub player: Pubkey, // Игрок
        pub token_mint: Pubkey, // Токен
        pub accumulated_winnings: u64, // Накопленная сумма выигрышей
        pub last_update: i64, // Последнее обновление
        pub bump: u8, // Bump для PDA
    }

    #[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
    pub struct LiquidityProvision {
        pub provider: Pubkey, // Адрес провайдера ликвидности
        pub amount: u64, // Количество предоставленных токенов
        pub timestamp: i64, // Время предоставления ликвидности
        pub withdrawn: bool, // Флаг вывода ликвидности
    }

    #[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
    pub struct ProviderReward {
        pub provider: Pubkey, // Адрес провайдера ликвидности
        pub accumulated_reward: u64, // Накопленное вознаграждение
    }

    #[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq)]
    pub enum RoundStatus {
        NotStarted, // Раунд не начат
        AcceptingBets, // Принимаем ставки
        BetsClosed, // Ставки закрыты (рулетка крутится на фронте)
        WaitingForRandom, // Ожидание генерации числа (можно опустить, если не нужно)
        Completed, // Раунд завершен, выигрыши можно забирать
    }

    #[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq)]
    pub enum BetType {
        Straight {
            number: u8,
        }, // Ставка на одно число
        Split {
            first: u8,
            second: u8,
        }, // Ставка на два соседних числа
        Corner {
            top_left: u8,
        }, // Ставка на 4 числа в углу
        Street {
            street: u8,
        }, // Ставка на 3 числа в ряду
        SixLine {
            six_line: u8,
        }, // Ставка на 6 чисел (2 ряда)
        FirstFour, // Ставка на первые 4 числа
        Red, // Ставка на красные числа
        Black, // Ставка на черные числа
        Even, // Ставка на четные числа
        Odd, // Ставка на нечетные числа
        Manque, // Ставка на 1-18
        Passe, // Ставка на 19-36
        Columns {
            column: u8,
        }, // Ставка на колонку
        P12, // Ставка на первую дюжину (1-12)
        M12, // Ставка на среднюю дюжину (13-24)
        D12, // Ставка на последнюю дюжину (25-36)
    }

    #[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
    pub struct Bet {
        pub amount: u64, // Без изменений
        pub bet_type: u8, // Вместо enum - просто числовой тип
        pub numbers: [u8; 4], // Массив всех возможных чисел
    }

    #[derive(Accounts)]
    pub struct InitializeVault<'info> {
        #[account(mut)]
        pub authority: Signer<'info>,

        /// CHECK: Это аккаунт минта SPL токена
        pub token_mint: AccountInfo<'info>,

        #[account(
            init,
            payer = authority,
            space = 8 + std::mem::size_of::<VaultAccount>() + 10000,
            seeds = [b"vault", token_mint.key().as_ref()],
            bump
        )]
        pub vault: Account<'info, VaultAccount>,

        /// CHECK: Это токен-аккаунт хранилища
        pub vault_token_account: AccountInfo<'info>,

        pub system_program: Program<'info, System>,
        pub token_program: Program<'info, Token>,
        pub rent: Sysvar<'info, Rent>,
    }

    #[derive(Accounts)]
    pub struct InitializeGameSession<'info> {
        #[account(mut)]
        pub authority: Signer<'info>,

        /// CHECK: This is the reward token mint
        pub reward_token_mint: AccountInfo<'info>,

        #[account(
            init,
            payer = authority,
            space = 8 + std::mem::size_of::<GameSession>() + 1000,
            seeds = [b"game_session"],
            bump
        )]
        pub game_session: Account<'info, GameSession>,

        pub system_program: Program<'info, System>,
        pub rent: Sysvar<'info, Rent>,
    }

    #[derive(Accounts)]
    pub struct ProvideLiquidity<'info> {
        #[account(mut)]
        pub vault: Account<'info, VaultAccount>,

        /// CHECK: Это токен-аккаунт провайдера ликвидности
        #[account(mut)]
        pub provider_token_account: AccountInfo<'info>,

        /// CHECK: Это токен-аккаунт хранилища
        #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
        pub vault_token_account: AccountInfo<'info>,

        #[account(mut)]
        pub liquidity_provider: Signer<'info>,

        pub token_program: Program<'info, Token>,
        pub system_program: Program<'info, System>,
    }

    #[derive(Accounts)]
    pub struct WithdrawLiquidity<'info> {
        #[account(mut)]
        pub vault: Account<'info, VaultAccount>,

        /// CHECK: Это токен-аккаунт провайдера ликвидности
        #[account(mut)]
        pub provider_token_account: AccountInfo<'info>,

        /// CHECK: Это токен-аккаунт хранилища
        #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
        pub vault_token_account: AccountInfo<'info>,

        #[account(mut)]
        pub liquidity_provider: Signer<'info>,

        pub token_program: Program<'info, Token>,
        pub system_program: Program<'info, System>,
    }

    #[derive(Accounts)]
    pub struct WithdrawProviderRevenue<'info> {
        #[account(mut)]
        pub vault: Account<'info, VaultAccount>,

        /// CHECK: Это токен-аккаунт провайдера ликвидности
        #[account(mut)]
        pub provider_token_account: AccountInfo<'info>,

        /// CHECK: Это токен-аккаунт хранилища
        #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
        pub vault_token_account: AccountInfo<'info>,

        #[account(mut)]
        pub liquidity_provider: Signer<'info>,

        pub token_program: Program<'info, Token>,
        pub system_program: Program<'info, System>,
    }

    #[derive(Accounts)]
    pub struct WithdrawOwnerRevenue<'info> {
        #[account(mut)]
        pub vault: Account<'info, VaultAccount>,

        /// CHECK: Это токен-аккаунт владельца
        #[account(mut)]
        pub owner_token_account: AccountInfo<'info>,

        /// CHECK: Это токен-аккаунт хранилища
        #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
        pub vault_token_account: AccountInfo<'info>,

        #[account(mut)]
        pub authority: Signer<'info>,

        pub token_program: Program<'info, Token>,
        pub system_program: Program<'info, System>,
    }

    #[derive(Accounts)]
    pub struct WithdrawAllWinnings<'info> {
        #[account(mut)]
        pub player_global_winnings: Account<'info, PlayerGlobalWinnings>,

        #[account(
        mut,
        constraint = vault.key() == player_global_winnings.token_mint,
    )]
        pub vault: Account<'info, VaultAccount>,

        /// CHECK: Токен-аккаунт игрока
        #[account(mut)]
        pub player_token_account: AccountInfo<'info>,

        /// CHECK: Токен-аккаунт хранилища
        #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
        pub vault_token_account: AccountInfo<'info>,

        #[account(
        mut,
        constraint = player.key() == player_global_winnings.player, // Проверка владельца
    )]
        pub player: Signer<'info>,

        pub token_program: Program<'info, Token>,
        pub system_program: Program<'info, System>,
    }

    #[derive(Accounts)]
    pub struct InitializeAndProvideLiquidity<'info> {
        #[account(mut)]
        pub authority: Signer<'info>,

        /// CHECK: Это аккаунт минта SPL токена
        pub token_mint: AccountInfo<'info>,

        #[account(
            init,
            payer = authority,
            space = 8 + std::mem::size_of::<VaultAccount>() + 10000,
            seeds = [b"vault", token_mint.key().as_ref()],
            bump
        )]
        pub vault: Account<'info, VaultAccount>,

        /// CHECK: Это токен-аккаунт провайдера ликвидности
        #[account(mut)]
        pub provider_token_account: AccountInfo<'info>,

        /// CHECK: Это токен-аккаунт хранилища
        pub vault_token_account: AccountInfo<'info>,

        #[account(mut)]
        pub liquidity_provider: Signer<'info>,

        pub system_program: Program<'info, System>,
        pub token_program: Program<'info, Token>,
        pub rent: Sysvar<'info, Rent>,
    }

    #[event]
    pub struct TotalWinningsWithdrawn {
        pub player: Pubkey,
        pub token_mint: Pubkey,
        pub amount: u64,
        pub timestamp: i64,
    }

    #[derive(Accounts)]
    pub struct StartNewRound<'info> {
        #[account(mut)]
        pub game_session: Account<'info, GameSession>,

        #[account(mut)]
        pub reward_vault: Account<'info, VaultAccount>,

        /// CHECK: This is the starter's token account
        #[account(mut)]
        pub starter_token_account: AccountInfo<'info>,

        /// CHECK: This is the reward vault's token account
        #[account(
        mut,
        constraint = reward_vault_token_account.key() == reward_vault.token_account,
    )]
        pub reward_vault_token_account: AccountInfo<'info>,

        #[account(mut)]
        pub starter: Signer<'info>,

        pub token_program: Program<'info, Token>,
        pub system_program: Program<'info, System>,
    }

    #[derive(Accounts)]
    pub struct PlaceBets<'info> {
        #[account(mut)]
        pub vault: Account<'info, VaultAccount>,

        #[account(mut)]
        pub game_session: Account<'info, GameSession>,

        /// CHECK: This is the player's token account
        #[account(mut)]
        pub player_token_account: AccountInfo<'info>,

        /// CHECK: This is the vault's token account
        #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
        pub vault_token_account: AccountInfo<'info>,

        #[account(mut)]
        pub player: Signer<'info>,

        #[account(
            init_if_needed,
            payer = player,
            space = 8 + std::mem::size_of::<PlayerBets>() + 1000,
            seeds = [
                b"player_bets",
                game_session.key().as_ref(),
                player.key().as_ref(),
                &game_session.current_round.to_le_bytes(),
            ],
            bump
        )]
        pub player_bets: Account<'info, PlayerBets>,

        pub token_program: Program<'info, Token>,
        pub system_program: Program<'info, System>,
        pub rent: Sysvar<'info, Rent>,
    }

    #[derive(Accounts)]
    pub struct CloseBets<'info> {
        #[account(mut)]
        pub game_session: Account<'info, GameSession>,

        #[account(mut)]
        pub reward_vault: Account<'info, VaultAccount>, // Добавлено

        #[account(mut)]
        pub vault: Account<'info, VaultAccount>,

        /// CHECK: This is the closer's token account
        #[account(mut)]
        pub closer_token_account: AccountInfo<'info>,

        /// CHECK: This is the reward vault's token account
        #[account(
        mut,
        constraint = reward_vault_token_account.key() == reward_vault.token_account,
    )]
        pub reward_vault_token_account: AccountInfo<'info>, // Добавлено

        /// CHECK: This is the vault's token account
        #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
        pub vault_token_account: AccountInfo<'info>,

        #[account(mut)]
        pub closer: Signer<'info>,

        pub token_program: Program<'info, Token>,
        pub system_program: Program<'info, System>,
    }

    #[derive(Accounts)]
    pub struct GetRandom<'info> {
        #[account(mut)]
        pub game_session: Account<'info, GameSession>,

        #[account(mut)]
        pub reward_vault: Account<'info, VaultAccount>,

        /// CHECK: Это токен-аккаунт инициатора генерации рандома
        #[account(mut)]
        pub initiator_token_account: AccountInfo<'info>,

        /// CHECK: Это токен-аккаунт хранилища вознаграждений
        #[account(
        mut,
        constraint = reward_vault_token_account.key() == reward_vault.token_account,
    )]
        pub reward_vault_token_account: AccountInfo<'info>,

        #[account(mut)]
        pub random_initiator: Signer<'info>,

        pub token_program: Program<'info, Token>,
        pub system_program: Program<'info, System>,
    }

    #[derive(Accounts)]
    pub struct ClaimWinnings<'info> {
        #[account(mut)]
        pub game_session: Account<'info, GameSession>,

        #[account(
        mut,
        constraint = player_bets.player == player.key(),
        seeds = [b"player_bets", game_session.key().as_ref(), player.key().as_ref(), &player_bets.round.to_le_bytes()],
        bump = player_bets.bump
    )]
        pub player_bets: Account<'info, PlayerBets>,

        #[account(
        mut,
        constraint = vault.key() == player_bets.vault,
    )]
        pub vault: Account<'info, VaultAccount>,

        #[account(
            init_if_needed,
            payer = player,
            space = 8 + std::mem::size_of::<PlayerGlobalWinnings>() + 100,
            seeds = [b"player_winnings", vault.token_mint.as_ref(), player.key().as_ref()],
            bump
        )]
        pub player_global_winnings: Account<'info, PlayerGlobalWinnings>,

        /// CHECK: Токен-аккаунт игрока
        #[account(mut)]
        pub player_token_account: AccountInfo<'info>,

        /// CHECK: Токен-аккаунт хранилища
        #[account(
        mut,
        constraint = vault_token_account.key() == vault.token_account,
    )]
        pub vault_token_account: AccountInfo<'info>,

        #[account(mut)]
        pub player: Signer<'info>,

        pub token_program: Program<'info, Token>,
        pub system_program: Program<'info, System>,
        pub rent: Sysvar<'info, Rent>,
    }

    #[event]
    pub struct RoundStarted {
        pub round: u64,
        #[index]
        pub starter: Pubkey,
        pub reward_token_mint: Pubkey,
        pub start_time: i64,
    }

    #[event]
    pub struct PayoutClaimed {
        pub round: u64,
        #[index]
        pub player: Pubkey,
        pub token_mint: Pubkey,
        pub amount: u64,
        pub timestamp: i64,
    }

    #[event]
    pub struct RoundCompleted {
        pub round: u64,
        #[index]
        pub completer: Pubkey,
        pub timestamp: i64,
    }

    #[event]
    pub struct BetsClosed {
        pub round: u64,
        #[index]
        pub closer: Pubkey,
        pub close_time: i64,
    }

    #[event]
    pub struct RandomGenerated {
        pub round: u64,
        #[index]
        pub initiator: Pubkey,
        pub winning_number: u8,
        pub generation_time: i64,
    }

    #[event]
    pub struct GameResult {
        #[index]
        pub player: Pubkey,
        pub token_mint: Pubkey,
        pub payout: u64,
        pub winning_number: u8,
        pub total_bet_amount: u64,
        pub timestamp: i64,
    }

    #[event]
    pub struct LiquidityProvided {
        #[index]
        pub provider: Pubkey,
        pub token_mint: Pubkey,
        pub amount: u64,
        pub timestamp: i64,
    }

    #[event]
    pub struct LiquidityWithdrawn {
        #[index]
        pub provider: Pubkey,
        pub token_mint: Pubkey,
        pub amount: u64,
        pub timestamp: i64,
    }

    #[event]
    pub struct ProviderRevenueWithdrawn {
        #[index]
        pub provider: Pubkey,
        pub token_mint: Pubkey,
        pub amount: u64,
        pub timestamp: i64,
    }

    #[event]
    pub struct OwnerRevenueWithdrawn {
        #[index]
        pub owner: Pubkey,
        pub token_mint: Pubkey,
        pub amount: u64,
        pub timestamp: i64,
    }

    #[event]
    pub struct BetsPlaced {
        pub player: Pubkey,
        pub token_mint: Pubkey,
        pub round: u64,
        pub bets: Vec<Bet>,
        pub total_amount: u64,
        pub timestamp: i64,
    }
}

#[error_code]
pub enum RouletteError {
    #[msg("Arithmetic overflow error")]
    ArithmeticOverflow,

    #[msg("Number of bets must be between 1 and 2")]
    InvalidNumberOfBets,

    #[msg("Insufficient funds for withdrawal")]
    InsufficientFunds,

    #[msg("Insufficient liquidity in vault")]
    InsufficientLiquidity,

    #[msg("Unauthorized access")]
    Unauthorized,

    #[msg("No reward available for withdrawal")]
    NoReward,

    #[msg("Must withdraw exact amount of liquidity")]
    MustWithdrawExactAmount,

    #[msg("Invalid bet")]
    InvalidBet,

    #[msg("Invalid token account")]
    InvalidTokenAccount,

    #[msg("Round is already in progress or waiting for random")]
    RoundInProgress,

    #[msg("Bets are not being accepted at this time")]
    BetsNotAccepted,

    #[msg("Round status does not allow this operation")]
    InvalidRoundStatus,

    #[msg("Too early to close bets, minimum time not elapsed")]
    TooEarlyToClose,

    #[msg("Too early for payouts, delay period not elapsed")]
    TooEarlyForPayouts,

    #[msg("Player has no bets in this round")]
    NoBetsInRound,

    #[msg("Game session not found")]
    GameSessionNotFound,

    #[msg("Invalid reward token")]
    InvalidRewardToken,

    #[msg("Vault mismatch for player bets")]
    VaultMismatch,

    #[msg("Cannot generate random number before closing bets")]
    RandomBeforeClosing,

    #[msg("Random number already generated for this round")]
    RandomAlreadyGenerated,

    #[msg("Payout already processed for this bet")]
    PayoutAlreadyProcessed,

    #[msg("Winning number not set for the current round")]
    WinningNumberNotSet,

    #[msg("Invalid player bets account")]
    InvalidPlayerBetsAccount,
}
