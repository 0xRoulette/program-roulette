

const { Connection, PublicKey, Keypair, Transaction, TransactionInstruction, sendAndConfirmTransaction, SystemProgram, SYSVAR_RENT_PUBKEY } = require('@solana/web3.js');
const fs = require('fs');
const borsh = require('@coral-xyz/borsh');

// --- КОНФИГУРАЦИЯ ---
const RPC_URL = 'https://api.devnet.solana.com'; // Или твой RPC URL, если другой
const PROGRAM_ID = new PublicKey(idl.address); 
const IDL_PATH = './roulette_game.json'; // <<< Путь к твоему IDL файлу (в той же папке)
// Путь к кошельку авторитета программы (!! Убедись, что этот файл существует на VPS !!)
const AUTHORITY_KEYPAIR_PATH = process.env.HOME + '/.config/solana/id.json';
// --- КОНЕЦ КОНФИГУРАЦИИ ---

// Подключение
const connection = new Connection(RPC_URL, 'confirmed');

// Загрузка кошелька авторитета
let authoritySigner;
try {
    const keypairData = Uint8Array.from(JSON.parse(fs.readFileSync(AUTHORITY_KEYPAIR_PATH)));
    authoritySigner = Keypair.fromSecretKey(keypairData);
    console.log('Authority Keypair loaded:', authoritySigner.publicKey.toBase58());
} catch (err) {
    console.error(`Failed to load authority keypair from ${AUTHORITY_KEYPAIR_PATH}:`, err);
    process.exit(1);
}

// Загрузка IDL
let idl;
try {
    idl = JSON.parse(fs.readFileSync(IDL_PATH, 'utf8'));
    console.log('IDL loaded successfully.');
} catch (err) {
    console.error(`Failed to load IDL from ${IDL_PATH}:`, err);
    process.exit(1);
}

// Вспомогательная функция для поиска дискриминатора (как в reset-game.js)
function findInstructionDiscriminator(idl, instructionName) {
    const instruction = idl.instructions.find(ix => ix.name === instructionName);
    if (!instruction || !instruction.discriminator) {
        throw new Error(`Discriminator for instruction "${instructionName}" not found in IDL`);
    }
    return Buffer.from(instruction.discriminator);
}

async function initializeGameSession() {
    try {
        // Находим PDA для game_session (используя правильный сид из IDL)
        const [gameSessionPda, gameSessionBump] = await PublicKey.findProgramAddress(
            [Buffer.from('game_session')], // <<< Правильный сид
            PROGRAM_ID
        );

        console.log('Attempting to initialize Game Session at PDA:', gameSessionPda.toBase58());
        console.log('Using authority:', authoritySigner.publicKey.toBase58());
        console.log('Using reward token mint:', REWARD_TOKEN_MINT.toBase58());

        // --- Ручное создание инструкции initialize_game_session ---
        const discriminator = findInstructionDiscriminator(idl, 'initialize_game_session');

        // Ключи согласно IDL для initialize_game_session
        const keys = [
            { pubkey: authoritySigner.publicKey, isSigner: true, isWritable: true },
            { pubkey: REWARD_TOKEN_MINT, isSigner: false, isWritable: false }, // <<< Добавлен reward_token_mint
            { pubkey: gameSessionPda, isSigner: false, isWritable: true },
            { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
            { pubkey: SYSVAR_RENT_PUBKEY, isSigner: false, isWritable: false }, // <<< Rent добавлен
        ];

        const instruction = new TransactionInstruction({
            keys: keys,
            programId: PROGRAM_ID,
            data: discriminator, // Нет аргументов у этой инструкции
        });
        // --- Конец ручного создания ---

        // Создаем и отправляем транзакцию
        const transaction = new Transaction().add(instruction);
        console.log("Sending initialize_game_session transaction...");

        const signature = await sendAndConfirmTransaction(
            connection,
            transaction,
            [authoritySigner] // Подписываем авторитетом
        );

        console.log('Транзакция выполнена успешно:', signature);
        console.log('Игровая сессия УСПЕШНО ИНИЦИАЛИЗИРОВАНА');
        console.log(`Ссылка на транзакцию: https://explorer.solana.com/tx/${signature}?cluster=devnet`); // <<< Укажи devnet или mainnet

    } catch (error) {
        console.error('Ошибка при инициализации игровой сессии:', error);
        if (error.logs) {
            console.error('Логи ошибки:', error.logs);
        }
        // Дополнительная диагностика
        if (error.message && error.message.includes("already in use")) {
            console.error("Аккаунт game_session, похоже, уже существует. Инициализация не требуется или используйте другой подход для сброса/обновления.");
        } else if (error.message && error.message.includes("custom program error")) {
            console.error("Получена ошибка программы:", error.message);
            // (можно добавить код для извлечения кода ошибки, как в reset-game.js)
        }
    }
}

initializeGameSession();
