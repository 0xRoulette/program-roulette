const { Connection, PublicKey, Keypair, Transaction, TransactionInstruction, sendAndConfirmTransaction } = require('@solana/web3.js');
const fs = require('fs');
const borsh = require('@coral-xyz/borsh'); // Нужен для кодирования дискриминатора

// Конфигурация
const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
const programId = new PublicKey('GZB6nqB9xSC8VKwWajtCu2TotPXz1mZCR5VwMLEKDj81');

// Загрузка кошелька (АВТОРИТЕТА ПРОГРАММЫ)
const keypairData = Uint8Array.from(JSON.parse(fs.readFileSync(process.env.HOME + '/.config/solana/id.json')));
const authoritySigner = Keypair.fromSecretKey(keypairData);

// Загрузка IDL - он все еще нужен для дискриминатора
const idl = JSON.parse(fs.readFileSync('./target/idl/roulette_game.json', 'utf8'));

// --- Вспомогательная функция для поиска дискриминатора ---
function findInstructionDiscriminator(idl, instructionName) {
    const instruction = idl.instructions.find(ix => ix.name === instructionName);
    if (!instruction || !instruction.discriminator) {
        throw new Error(`Discriminator for instruction "${instructionName}" not found in IDL`);
    }
    // Дискриминатор в IDL уже представлен как массив байт
    return Buffer.from(instruction.discriminator);
}


async function resetGameSession() {
  try {
    // Находим PDA для game_session (точно так же, как при инициализации)
    const [gameSessionPda] = await PublicKey.findProgramAddress(
      [Buffer.from('game_session_v1')],
      programId // Используем programId напрямую
    );

    console.log('Attempting to reset Game Session at PDA:', gameSessionPda.toBase58());
    console.log('Using authority:', authoritySigner.publicKey.toBase58());

    // --- Ручное создание инструкции ---
    // 1. Получаем дискриминатор из IDL
    const discriminator = findInstructionDiscriminator(idl, 'reset_game_session');

    // 2. Определяем ключи согласно структуре ResetGameSession в Rust
    const keys = [
        { pubkey: gameSessionPda, isSigner: false, isWritable: true },
        { pubkey: authoritySigner.publicKey, isSigner: true, isWritable: true }, // Authority - signer и плательщик (mut)
    ];

    // 3. Создаем инструкцию
    const instruction = new TransactionInstruction({
        keys: keys,
        programId: programId,
        data: discriminator, // Данные - только дискриминатор, так как нет аргументов
    });
    // --- Конец ручного создания ---


    // Создаем и отправляем транзакцию
    const transaction = new Transaction().add(instruction);
    console.log("Sending reset transaction...");

    const signature = await sendAndConfirmTransaction(
      connection,
      transaction,
      [authoritySigner] // Подписываем авторитетом
    );

    console.log('Транзакция выполнена успешно:', signature);
    console.log('Игровая сессия СБРОШЕНА');
    console.log(`Ссылка на транзакцию: https://explorer.solana.com/tx/${signature}?cluster=devnet`);

  } catch (error) {
    console.error('Ошибка при сбросе игровой сессии:', error);
    if (error.logs) {
      console.error('Логи ошибки:', error.logs);
    }
     // Дополнительная диагностика
     if (error.message && error.message.includes("Account does not exist")) {
         console.error("Возможно, аккаунт game_session не существует. Попробуйте сначала запустить init-game.js");
     } else if (error.message && error.message.includes("custom program error")) {
         console.error("Получена ошибка программы:", error.message);
         // Попробуем извлечь код ошибки, если он есть в логах
         const customErrorRegex = /Custom program error: (0x[a-fA-F0-9]+)/;
         if (error.logs) {
             for (const log of error.logs) {
                 const match = log.match(customErrorRegex);
                 if (match) {
                     console.error(`   -> Код ошибки Anchor: ${match[1]}`);
                     // Тут можно добавить расшифровку кодов ошибок, если есть map
                     break;
                 }
             }
         }
     }
  }
}

// Перед запуском убедись, что @coral-xyz/borsh установлен:
// npm install @coral-xyz/borsh
resetGameSession();