// program-roulette/tests/roulette-game.ts
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { RouletteGame } from "../target/types/roulette_game"; // Путь к типам IDL
import { assert } from "chai";
import { PublicKey, SystemProgram, Keypair, SYSVAR_RENT_PUBKEY } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID, createMint, createAssociatedTokenAccount, mintTo, getAccount, createAssociatedTokenAccountInstruction } from "@solana/spl-token";
import { BN } from "bn.js";

describe("roulette-game", () => {
  // Configure the client to use the local cluster.
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.RouletteGame as Program<RouletteGame>;
  const payer = provider.wallet as anchor.Wallet; // Используем кошелек из Anchor.toml

  // Ключи для тестов
  const mintAuthority = Keypair.generate();
  let tokenMint: PublicKey;
  const player = payer; // Для простоты используем плательщика как игрока
  let playerTokenAccount: PublicKey;
  let vaultPda: PublicKey;
  let vaultTokenAccount: PublicKey; // ATA для vaultPda
  let gameSessionPda: PublicKey;
  let playerBetsPda: PublicKey;
  let claimRecordPda: PublicKey;
  const roundToCheck = new BN(1);

  before(async () => {
    // --- Шаг 0: Подготовка токенов ---
    // Airdrop SOL для создания mint
    await provider.connection.requestAirdrop(mintAuthority.publicKey, 1 * anchor.web3.LAMPORTS_PER_SOL);
    await new Promise(resolve => setTimeout(resolve, 500)); // Даем время на обработку airdrop

    tokenMint = await createMint(
      provider.connection,
      mintAuthority, // payer for mint
      mintAuthority.publicKey, // mint authority
      null, // freeze authority
      9 // decimals
    );
    console.log("Token Mint:", tokenMint.toBase58());

    playerTokenAccount = await createAssociatedTokenAccount(
      provider.connection,
      payer.payer, // payer for ATA
      tokenMint,
      player.publicKey
    );
    console.log("Player ATA:", playerTokenAccount.toBase58());

    // Минтим токены игроку
    await mintTo(
      provider.connection,
      payer.payer, // payer
      tokenMint,
      playerTokenAccount,
      mintAuthority, // mint authority
      1_000_000_000_000 // 1000 токенов с 9 децималами
    );
    console.log("Minted 1000 tokens to player ATA");

    // --- Шаг 1: Найти PDAs ---
    [gameSessionPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("game_session")],
      program.programId
    );
    [playerBetsPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("player_bets"), gameSessionPda.toBuffer(), player.publicKey.toBuffer()],
      program.programId
    );
    [vaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), tokenMint.toBuffer()],
      program.programId
    );
    // Находим ATA для Vault PDA
    vaultTokenAccount = await anchor.utils.token.associatedAddress({
      mint: tokenMint,
      owner: vaultPda,
    });
    console.log("Vault PDA:", vaultPda.toBase58());
    console.log("Vault ATA:", vaultTokenAccount.toBase58());

    // Claim record PDA (требует номера раунда, найдем позже, если понадобится,
    // но для вызова claim_my_winnings он будет создан init_if_needed)
    const roundBytes = roundToCheck.toArrayLike(Buffer, 'le', 8);
    [claimRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("claim_record"), player.publicKey.toBuffer(), roundBytes],
      program.programId
    );
    console.log("Claim Record PDA (for round 1):", claimRecordPda.toBase58());

    // --- Шаг 2: Инициализация (если еще не сделано другими тестами) ---
    try {
      await program.methods.initializeGameSession().accounts({
        authority: payer.publicKey,
        gameSession: gameSessionPda,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      }).rpc();
      console.log("Game Session initialized.");
    } catch (e) { if (!e.toString().includes("already in use")) throw e; console.log("Game Session already initialized."); }

    // --- ВОЗВРАЩАЕМСЯ К ANCHOR METHODS ДЛЯ ИНИЦИАЛИЗАЦИИ VAULT --- 
    // --- С ЯВНЫМ СОЗДАНИЕМ ATA ПЕРЕД ВЫЗОВОМ --- 
    try {
        // 1. Явно создаем ATA для Vault PDA 
        console.log("Attempting to create Vault ATA explicitly before initializeVault...");
        await createAssociatedTokenAccount(
            provider.connection,
            payer.payer, // Используем signer/payer для создания ATA 
            tokenMint,
            vaultPda,    // Владелец - Vault PDA
            true         // allowOwnerOffCurve: true для PDA
        );
        console.log("Explicit Vault ATA creation/check successful (or already existed):", vaultTokenAccount.toBase58());
    } catch(ataError) {
        // Игнорируем ошибку, если ATA уже существует, иначе выбрасываем 
        if (ataError.message && !ataError.message.includes("TokenAccountAlreadyAssociatedWithMint")) {
            console.error("Error explicitly creating Vault ATA:", ataError);
            throw ataError; // Перебрасываем непредвиденную ошибку ATA
        } else {
            console.log("Vault ATA likely already existed.");
        }
    }

    // 2. Инициализируем Vault с помощью program.methods 
    try {
        console.log("Attempting to initialize Vault using program.methods...");
        // Используем camelCase для имен аккаунтов согласно IDL 
        await program.methods.initializeVault()
            .accounts({
                authority: payer.publicKey,
                tokenMint: tokenMint,
                vault: vaultPda, // PDA аккаунт для инициализации
                vaultTokenAccount: vaultTokenAccount, // Адрес *существующего* ATA 
                treasurySolAccount: payer.publicKey, // Адрес казны (используем payer)
                systemProgram: SystemProgram.programId,
                tokenProgram: TOKEN_PROGRAM_ID,
                rent: SYSVAR_RENT_PUBKEY,
            })
            .rpc(); // Anchor обработает подпись authority (payer)
        console.log("Vault initialized using program.methods.");

    } catch (e) {
        // Обрабатываем ошибку, если хранилище уже инициализировано 
        if (!e.toString().includes("already in use") && !e.toString().includes("custom program error: 0x0")) {
            console.error("Error initializing vault with program.methods:", e);
            if (e.logs) { console.error("Logs:", e.logs); } // Показываем логи, если есть
            throw e; // Перебрасываем другие ошибки
        }
        console.log("Vault likely already initialized (program.methods).");
    }

    // Теперь добавляем ликвидность отдельно (этот блок остается с program.methods)
    try {
        console.log("Attempting to provide initial liquidity...");
        await program.methods.provideLiquidity(new BN(1_000_000_000_000))
            .accounts({
                vault: vaultPda,
                providerTokenAccount: playerTokenAccount,
                vaultTokenAccount: vaultTokenAccount,
                liquidityProvider: player.publicKey,
                tokenProgram: TOKEN_PROGRAM_ID, // Оставляем, если линтер не ругается; Убираем, если ругается
            })
            .rpc();
        console.log("Initial liquidity provided.");
    } catch (liqError) {
        console.error("Failed to provide initial liquidity:", liqError);
        // Можно добавить проверку на случай, если ликвидность уже была добавлена
    }
    // --- КОНЕЦ ИЗМЕНЕННОГО БЛОКА ---


    try {
      // Используем camelCase для имен аккаунтов
      await program.methods.initializePlayerBets().accounts({
        player: player.publicKey,
        gameSession: gameSessionPda,
        playerBets: playerBetsPda,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      }).rpc();
      console.log("Player Bets initialized.");
    } catch (e) { if (!e.toString().includes("already in use")) throw e; console.log("Player Bets already initialized."); }

  });

  it("Claims winnings successfully after a winning bet", async () => {
    // --- Шаг 3: Провести раунд с выигрышной ставкой ---
    console.log("Starting round 1...");
    // Используем camelCase для имен аккаунтов
    await program.methods.startNewRound().accounts({
      gameSession: gameSessionPda,
      starter: player.publicKey,
      systemProgram: SystemProgram.programId,
    }).rpc();
    let gameSession = await program.account.gameSession.fetch(gameSessionPda);
    assert.ok(gameSession.currentRound.eq(roundToCheck), "Round number should be 1");
    assert.equal(gameSession.roundStatus.hasOwnProperty("acceptingBets"), true, "Round status should be AcceptingBets");

    console.log("Placing a winning bet (e.g., on Red)...");
    const betAmount = new BN(100_000_000); // 0.1 токена
    const betOnRed = {
      amount: betAmount,
      betType: 6, // BET_TYPE_RED
      numbers: [0, 0, 0, 0], // Не используются для Red
    };
    // Используем camelCase для имен аккаунтов
    await program.methods.placeBet(betOnRed).accounts({
      vault: vaultPda,
      gameSession: gameSessionPda,
      playerTokenAccount: playerTokenAccount,
      vaultTokenAccount: vaultTokenAccount,
      player: player.publicKey,
      playerBets: playerBetsPda,
      tokenProgram: TOKEN_PROGRAM_ID,
    }).rpc();
    console.log("Bet placed.");

    // Проверка баланса игрока после ставки
    let playerTokenAccData = await getAccount(provider.connection, playerTokenAccount);
    console.log("Player balance after bet:", playerTokenAccData.amount.toString()); // Ожидаем 999.9 токенов

    console.log("Closing bets (requires waiting MIN_ROUND_DURATION locally - skipping wait for test)...");
    // В тесте можно не ждать MIN_ROUND_DURATION, но нужно быть уверенным, что статус изменится
    // Force close bets (NOTE: This bypasses time check, only for testing!)
    try {
      // Прямой вызов closeBets
      // Используем camelCase для имен аккаунтов
      await program.methods.closeBets().accounts({
        gameSession: gameSessionPda,
        closer: player.publicKey,
        systemProgram: SystemProgram.programId,
      }).rpc();
      console.log("Bets closed.");
    } catch (e) {
      console.error("Failed to close bets:", e);
      // Попробуем обновить статус вручную, если closeBets не сработал (хак для теста)
      // gameSession = await program.account.gameSession.fetch(gameSessionPda);
      // gameSession.roundStatus = { betsClosed: {} }; // Это не сработает, нельзя менять состояние так
      // console.warn("Could not close bets via RPC, proceeding...");
      throw e; // Прерываем тест, если закрыть не удалось
    }

    gameSession = await program.account.gameSession.fetch(gameSessionPda);
    assert.equal(gameSession.roundStatus.hasOwnProperty("betsClosed"), true, "Round status should be BetsClosed");


    console.log("Getting random number...");
    // Используем camelCase для имен аккаунтов
    await program.methods.getRandom().accounts({
      gameSession: gameSessionPda,
      randomInitiator: player.publicKey,
    }).rpc();
    gameSession = await program.account.gameSession.fetch(gameSessionPda);
    assert.equal(gameSession.roundStatus.hasOwnProperty("completed"), true, "Round status should be Completed");
    assert.ok(gameSession.winningNumber !== null, "Winning number should exist");
    console.log(`Winning number: ${gameSession.winningNumber}`);
    assert.ok(gameSession.lastCompletedRound.eq(roundToCheck), "Last completed round should be 1");


    // --- Шаг 4: Попытка клейма ---
    console.log("Attempting to claim winnings...");
    const balanceBeforeClaim = (await getAccount(provider.connection, playerTokenAccount)).amount;

    try {
      // !!! ОШИБКА ЛИНТЕРА: Property 'claimMyWinnings' does not exist... (ОЖИДАЕМО)
      // Используем camelCase для имен аккаунтов
      const txSignature = await program.methods.claimMyWinnings().accounts({
        player: player.publicKey,
        gameSession: gameSessionPda,
        playerBets: playerBetsPda,
        vault: vaultPda, // Используем vault из PlayerBets (они должны совпадать)
        vaultTokenAccount: vaultTokenAccount,
        playerTokenAccount: playerTokenAccount,
        claimRecord: claimRecordPda, // Будет создан, если нужно
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      }).rpc();
      console.log("Claim winnings transaction successful:", txSignature);

      // --- Шаг 5: Проверка результата ---
      const balanceAfterClaim = (await getAccount(provider.connection, playerTokenAccount)).amount;
      console.log("Balance before claim:", balanceBeforeClaim.toString());
      console.log("Balance after claim: ", balanceAfterClaim.toString());

      // Проверяем, что баланс увеличился. Точная сумма зависит от выигрышного номера.
      assert.ok(balanceAfterClaim > balanceBeforeClaim, "Player balance should increase after claiming winnings.");

      // Проверяем, что ClaimRecord создан и помечен как claimed
      // !!! ОШИБКА ЛИНТЕРА: Property 'claimRecord' does not exist... (ОЖИДАЕМО)
      const claimRecordAccount = await program.account.claimRecord.fetch(claimRecordPda);
      assert.isTrue(claimRecordAccount.claimed, "Claim record should be marked as claimed");
      console.log("Claim record is marked as claimed.");


    } catch (error) {
      console.error("Claim winnings failed:", error);
      // Попробуем извлечь ошибку программы, если она есть
      if (error.logs) {
        console.error("Program Logs on Failure:", error.logs);
        const anchorErrorRegex = /Program log: AnchorError occurred. Error Code: (\w+). Error Number: (\d+). Error Message: (.*)./;
        let foundError = false;
        for (const log of error.logs) {
          const anchorMatch = log.match(anchorErrorRegex);
          if (anchorMatch) {
            const errorCodeName = anchorMatch[1];
            const errorCodeNum = parseInt(anchorMatch[2]);
            const errorMsg = anchorMatch[3];
            console.error(`Detected Program Error: ${errorCodeName} (${errorCodeNum}) - ${errorMsg}`);
            foundError = true;
            // Если ошибка NoWinningsFound, то тест должен упасть, если мы ОЖИДАЛИ выигрыш
            if (errorCodeName === 'NoWinningsFound') {
              assert.fail("Claim failed with NoWinningsFound, but a winning bet was placed.");
            }
            break;
          }
        }
        if (!foundError) {
          console.error("Could not parse specific program error from logs.");
        }
      }
      assert.fail(`Claim winnings failed unexpectedly: ${error}`);
    }
  });

  // Можно добавить другие тесты, например, на попытку повторного клейма,
  // клейм при отсутствии выигрыша (должен вернуть NoWinningsFound) и т.д.

});