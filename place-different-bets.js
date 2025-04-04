const { Connection, PublicKey, Keypair, Transaction, TransactionInstruction, SystemProgram, SYSVAR_RENT_PUBKEY } = require('@solana/web3.js');
const BN = require('bn.js');
const fs = require('fs');
const borsh = require('borsh'); // Добавляем библиотеку для сериализации
const { TOKEN_PROGRAM_ID, getAssociatedTokenAddress, createAssociatedTokenAccountInstruction } = require('@solana/spl-token');


// --- Константы ---
const PROGRAM_ID = new PublicKey('GZB6nqB9xSC8VKwWajtCu2TotPXz1mZCR5VwMLEKDj81');
const TOKEN_MINT = new PublicKey('G6ewh3aD36fYNd3t5bGthDTeczH5kowNzBoypbf48XUM'); // Ваш токен
const TOKEN_DECIMALS = 9;

// --- Типы ставок (соответствуют вашему enum/const в Rust) ---
const BET_TYPES = {
    STRAIGHT: 0,  // Ставка на одно число
    SPLIT: 1,     // Ставка на два смежных числа
    CORNER: 2,    // Ставка на четыре числа, образующие квадрат
    STREET: 3,    // Ставка на три числа в ряд
    SIXLINE: 4,   // Ставка на шесть чисел в двух рядах
    FIRSTFOUR: 5, // Ставка на первые четыре числа
    RED: 6,       // Ставка на красные числа
    BLACK: 7,     // Ставка на черные числа
    EVEN: 8,      // Ставка на четные числа
    ODD: 9,       // Ставка на нечетные числа
    MANQUE: 10,   // Ставка на числа 1-18
    PASSE: 11,    // Ставка на числа 19-36
    COLUMN: 12,   // Ставка на колонку
    P12: 13,      // Первая дюжина (1-12)
    M12: 14,      // Вторая дюжина (13-24)
    D12: 15       // Третья дюжина (25-36)
};

// --- Сначала объявляем класс Bet ---
class Bet {
    constructor({ amount, bet_type, numbers }) {
        this.amount = new BN(amount);
        this.bet_type = bet_type;
        // Убедимся, что numbers - это массив из 4 чисел
        const fixedNumbers = Array.isArray(numbers) ? numbers : [0, 0, 0, 0];
        while (fixedNumbers.length < 4) {
            fixedNumbers.push(0);
        }
        this.numbers = Uint8Array.from(fixedNumbers.slice(0, 4));
    }
}

const BetSchemaStruct = {
    kind: 'struct',
    fields: [
        ['amount', 'u64'],
        ['bet_type', 'u8'],
        ['numbers', ['u8', 4]] // Фиксированный массив u8 длиной 4
    ]
};



// --- Функция для загрузки кошелька ---
const loadWalletKey = (keypairFile) => {
    try {
        const keypairData = fs.readFileSync(keypairFile, 'utf-8');
        const keypairJson = JSON.parse(keypairData);
        return Keypair.fromSecretKey(new Uint8Array(keypairJson));
    } catch (error) {
        console.error('Ошибка при загрузке кошелька:', error);
        throw error;
    }
};

// --- Основная функция для размещения ставки ---
async function placeSingleBet(betType, numbers, amountInTokens) {
    try {
        // Загружаем кошелек
        const wallet = loadWalletKey(process.env.HOME + '/.config/solana/id.json');
        console.log('Кошелек загружен:', wallet.publicKey.toString());

        // Подключаемся к сети
        const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
        console.log('Подключение установлено');

        // Вычисляем адрес GameSession PDA
        const [gameSessionPda] = await PublicKey.findProgramAddress(
            [Buffer.from('game_session')],
            PROGRAM_ID
        );
        console.log('Game Session PDA:', gameSessionPda.toString());

        // Получаем информацию о GameSession
        const gameSessionAccount = await connection.getAccountInfo(gameSessionPda);
        if (!gameSessionAccount) {
            throw new Error(`Аккаунт игровой сессии (${gameSessionPda}) не найден`);
        }

        // Извлекаем текущий раунд и статус (смещения могут отличаться, сверьтесь с Rust структурой)
        // Примерные смещения: 8 байт дискриминатор, 8 байт current_round, 8 байт start_time, 1 байт status
        const accountData = gameSessionAccount.data;
        const currentRound = new BN(accountData.slice(8, 16), 'le').toNumber();
        const roundStatus = accountData[24]; // Проверьте это смещение!

        console.log('Текущий раунд:', currentRound);
        console.log('Статус раунда:', roundStatus === 1 ? 'AcceptingBets' : `Не принимает ставки (статус ${roundStatus})`);

        if (roundStatus !== 1) { // 1 соответствует AcceptingBets
            throw new Error('Текущий раунд не принимает ставки!');
        }

        // Вычисляем адрес Vault PDA для нашего токена
        const [vaultPda] = await PublicKey.findProgramAddress(
            [Buffer.from('vault'), TOKEN_MINT.toBuffer()],
            PROGRAM_ID
        );
        console.log('Vault PDA:', vaultPda.toString());

        // Находим ATA игрока для токена
        const playerTokenAccount = await getAssociatedTokenAddress(
            TOKEN_MINT,
            wallet.publicKey
        );
        console.log('Player Token Account:', playerTokenAccount.toString());

        // Находим ATA хранилища для токена
        const vaultTokenAccount = await getAssociatedTokenAddress(
            TOKEN_MINT,
            vaultPda,
            true // allowOwnerOffCurve для PDA
        );
        console.log('Vault Token Account:', vaultTokenAccount.toString());

        // --- Проверка и подготовка к созданию ATA ---
        console.log("Проверка существования токен-аккаунтов...");
        const instructions = []; // Массив для инструкций

        const playerAtaInfo = await connection.getAccountInfo(playerTokenAccount);
        if (!playerAtaInfo) {
            console.log(`ATA игрока (${playerTokenAccount}) не найден. Добавляем инструкцию для создания...`);
            instructions.push(
                createAssociatedTokenAccountInstruction(
                    wallet.publicKey, // payer (кто платит за создание)
                    playerTokenAccount, // ata address
                    wallet.publicKey, // owner
                    TOKEN_MINT          // mint
                )
            );
        } else {
            console.log("ATA игрока существует.");
        }

        const vaultAtaInfo = await connection.getAccountInfo(vaultTokenAccount);
        if (!vaultAtaInfo) {
            // Эта проверка остается, т.к. скрипт не должен создавать ATA хранилища
            console.error(`ОШИБКА: ATA хранилища (${vaultTokenAccount}) для токена ${TOKEN_MINT} не найден!`);
            console.error(`Возможно, ликвидность для этого токена еще не предоставлялась.`);
            return false; // Прерываем выполнение
        }
        console.log("ATA хранилища существует.");
        // --- Конец проверки ---


        // Находим PlayerBets PDA
        const currentRoundBuffer = Buffer.alloc(8);
        currentRoundBuffer.writeBigUInt64LE(BigInt(currentRound));

        const [playerBetsPda] = await PublicKey.findProgramAddress(
            [
                Buffer.from('player_bets'),
                gameSessionPda.toBuffer(),
                wallet.publicKey.toBuffer(),
                currentRoundBuffer // Используем сериализованный номер раунда
            ],
            PROGRAM_ID
        );
        console.log('Player Bets PDA:', playerBetsPda.toString());

        // --- Подготовка данных для инструкции ---

        // 1. Дискриминатор для place_bets (из IDL)
        const placeBetsDiscriminator = Buffer.from([49, 131, 14, 212, 212, 143, 224, 150]);

        // 2. Вектор ставок (в нашем случае из одной ставки)
        const betAmountLamports = new BN(amountInTokens).mul(new BN(10).pow(new BN(TOKEN_DECIMALS)));

        const betToSerialize = new Bet({
            amount: betAmountLamports,
            bet_type: betType,
            numbers: numbers
        });

        console.log(`Детали ставки для сериализации:`);
        console.log(`- Тип: ${betToSerialize.bet_type}`);
        console.log(`- Числа: [${betToSerialize.numbers.join(', ')}]`);
        console.log(`- Сумма (lamports): ${betToSerialize.amount.toString()}`);

        // --- Ручная сериализация структуры Bet ---
        console.log('Начинаем ручную сериализацию Bet...');

        // 1. amount (u64, 8 байт, little-endian)
        const amountBuffer = betToSerialize.amount.toArrayLike(Buffer, 'le', 8);
        console.log('  Amount Buffer (hex):', amountBuffer.toString('hex'));

        // 2. bet_type (u8, 1 байт)
        const betTypeBuffer = Buffer.from([betToSerialize.bet_type]);
        console.log('  Bet Type Buffer (hex):', betTypeBuffer.toString('hex'));

        // 3. numbers ([u8; 4], 4 байта) - уже Uint8Array(4)
        const numbersBuffer = Buffer.from(betToSerialize.numbers);
        console.log('  Numbers Buffer (hex):', numbersBuffer.toString('hex'));

        // Объединяем буферы для одной ставки
        const serializedBet = Buffer.concat([amountBuffer, betTypeBuffer, numbersBuffer]);
        console.log('Ручной serializedBet (hex):', serializedBet.toString('hex'));
        console.log('Длина serializedBet:', serializedBet.length, '(ожидается 8 + 1 + 4 = 13)');
        // --- Конец ручной сериализации ---

        // Формируем вектор: длина вектора (4 байта LE) + сериализованная ставка
        const betsVectorBuffer = Buffer.concat([
            Buffer.from(new BN(1).toArray('le', 4)), // Длина вектора = 1
            serializedBet
        ]);

        // 3. Объединяем дискриминатор и вектор ставок
        const instructionData = Buffer.concat([placeBetsDiscriminator, betsVectorBuffer]);
        console.log('Instruction Data (hex):', instructionData.toString('hex'));

        // --- Создание инструкции ---
        instructions.push(
            new TransactionInstruction({
               keys: [
                   // Порядок и isWritable/isSigner согласно IDL для PlaceBets
                   { pubkey: vaultPda, isSigner: false, isWritable: true },
                   { pubkey: gameSessionPda, isSigner: false, isWritable: true },
                   { pubkey: playerTokenAccount, isSigner: false, isWritable: true },
                   { pubkey: vaultTokenAccount, isSigner: false, isWritable: true },
                   { pubkey: wallet.publicKey, isSigner: true, isWritable: true }, // player
                   { pubkey: playerBetsPda, isSigner: false, isWritable: true }, // player_bets
                   { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
                   { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
                   { pubkey: SYSVAR_RENT_PUBKEY, isSigner: false, isWritable: false },
               ],
               programId: PROGRAM_ID,
               data: instructionData
           })
       );

        // --- Создание и отправка транзакции ---
// Строка 245
const transaction = new Transaction().add(...instructions); // Используем массив instructions со spread оператором (...)
        transaction.feePayer = wallet.publicKey;

        const blockHash = await connection.getLatestBlockhash();
        transaction.recentBlockhash = blockHash.blockhash;
        transaction.sign(wallet); // Подписываем только кошельком игрока

        console.log('Отправка транзакции со ставкой...');
        const txid = await connection.sendRawTransaction(
            transaction.serialize(),
            { skipPreflight: false, preflightCommitment: 'confirmed' }
        );

        console.log('Транзакция отправлена, ID:', txid);
        console.log(`Ссылка на транзакцию: https://explorer.solana.com/tx/${txid}?cluster=devnet`);

        const confirmation = await connection.confirmTransaction({
            blockhash: blockHash.blockhash,
            lastValidBlockHeight: blockHash.lastValidBlockHeight,
            signature: txid
        }, 'confirmed');

        if (confirmation.value.err) {
            console.error('Ошибка в транзакции:', confirmation.value.err);
            console.error("Логи:", confirmation.value.logs);
            return false;
        } else {
            console.log('Ставка успешно размещена!');
            return true;
        }

    } catch (error) {
        console.error('Критическая ошибка при размещении ставки:', error);
        if (error.logs) {
            console.error('Логи транзакции:', error.logs);
        }
        return false;
    }
}

// --- Вызов функции для ставки "Четное" (EVEN = 8) ---
// Сумма ставки 10000 токенов
// Для ставок типа "Четное", "Красное" и т.д., конкретные числа не важны, передаем нули.
placeSingleBet(BET_TYPES.EVEN, [0, 0, 0, 0], 10000);