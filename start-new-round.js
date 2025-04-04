const { Connection, PublicKey, Keypair, Transaction, TransactionInstruction, SystemProgram } = require('@solana/web3.js'); // Добавляем SystemProgram
const { TOKEN_PROGRAM_ID, getAssociatedTokenAddress } = require('@solana/spl-token'); // Добавляем импорты из spl-token
const fs = require('fs');

// Константы программы
const PROGRAM_ID = new PublicKey('GZB6nqB9xSC8VKwWajtCu2TotPXz1mZCR5VwMLEKDj81');
const GAME_SESSION = new PublicKey('7f669mCiZgknfauQBoJwqcyzmSyQ3NeKvRnv5iz4zMMx');
const REWARD_TOKEN_MINT = new PublicKey('4QhhRXq8NvyjpxLiC44nEFzKX9ZRYr9twMvBr7Jm7cLb');

// Функция для загрузки ключа кошелька
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

async function startNewRound() {
    try {
        // Загружаем кошелек администратора
        const wallet = loadWalletKey(process.env.HOME + '/.config/solana/id.json');
        console.log('Кошелек загружен:', wallet.publicKey.toString());

        // Подключаемся к сети
        const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
        console.log('Подключение установлено');

        // Проверка текущего статуса раунда
        const gameSessionAccount = await connection.getAccountInfo(GAME_SESSION);
        if (!gameSessionAccount) {
            throw new Error('Аккаунт игровой сессии не найден');
        }

        // Первые 8 байт - дискриминатор, затем 8 байт current_round, затем 1 байт round_status
        const roundStatus = gameSessionAccount.data[16];
        const statusMap = {
            0: 'NotStarted',
            1: 'AcceptingBets',
            2: 'BetsClosed',
            3: 'WaitingForRandom',
            4: 'Completed'
        };
        // Строки 45-54
        console.log('Текущий статус раунда:', statusMap[roundStatus] || 'Неизвестный статус');

        // Дискриминатор для start_new_round из IDL
        // Используем значение из IDL файла roulette_game.json
        const discriminator = Buffer.from([180, 48, 50, 160, 186, 163, 79, 185]); // <<< ЗАМЕНЯЕМ ЗДЕСЬ

        console.log('Создание инструкции start_new_round...');

        // Находим PDA для хранилища токена вознаграждения
        const [rewardVaultPda] = await PublicKey.findProgramAddress(
            [Buffer.from("vault"), REWARD_TOKEN_MINT.toBuffer()],
            PROGRAM_ID
        );
        console.log('Reward Vault PDA:', rewardVaultPda.toString());

        // Находим ATA стартера для токена вознаграждения
        const starterTokenAccount = await getAssociatedTokenAddress(
            REWARD_TOKEN_MINT,
            wallet.publicKey
        );
        console.log('Starter Token Account:', starterTokenAccount.toString());

        // Находим ATA хранилища для токена вознаграждения (PDA может быть владельцем ATA)
        const rewardVaultTokenAccount = await getAssociatedTokenAddress(
            REWARD_TOKEN_MINT,
            rewardVaultPda,
            true // allowOwnerOffCurve: true для PDA
        );
        console.log('Reward Vault Token Account:', rewardVaultTokenAccount.toString());


        // Создаем инструкцию start_new_round с правильными аккаунтами
        const instruction = new TransactionInstruction({
            keys: [
                // Соответствует структуре StartNewRound в lib.rs
                { pubkey: GAME_SESSION, isSigner: false, isWritable: true },
                { pubkey: rewardVaultPda, isSigner: false, isWritable: true },
                { pubkey: starterTokenAccount, isSigner: false, isWritable: true },
                { pubkey: rewardVaultTokenAccount, isSigner: false, isWritable: true },
                { pubkey: wallet.publicKey, isSigner: true, isWritable: true }, // Стартер - подписывающий и плательщик
                { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
                { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
            ],
            programId: PROGRAM_ID,
            data: discriminator // Убедитесь, что дискриминатор верный!
        });

        // Создаем транзакцию
        const transaction = new Transaction().add(instruction);
        transaction.feePayer = wallet.publicKey;

        // Получаем последний блокхеш
        const blockHash = await connection.getLatestBlockhash();
        transaction.recentBlockhash = blockHash.blockhash;

        // Подписываем транзакцию
        transaction.sign(wallet);

        // Отправляем транзакцию
        console.log('Отправка транзакции для запуска нового раунда...');
        const serializedTransaction = transaction.serialize();

        const txid = await connection.sendRawTransaction(
            serializedTransaction,
            { skipPreflight: false, preflightCommitment: 'confirmed' }
        );

        console.log('Транзакция отправлена, ID:', txid);
        console.log(`Ссылка на транзакцию: https://explorer.solana.com/tx/${txid}?cluster=devnet`);

        // Ждем подтверждения
        const confirmation = await connection.confirmTransaction({
            blockhash: blockHash.blockhash,
            lastValidBlockHeight: blockHash.lastValidBlockHeight,
            signature: txid
        }, 'confirmed');

        console.log('Транзакция подтверждена!');
        console.log('Статус:', confirmation.value.err ? 'Ошибка' : 'Успех');

        if (confirmation.value.err) {
            console.error('Ошибка в транзакции:', confirmation.value.err);
        } else {
            console.log('Новый раунд успешно запущен!');

            // Проверяем статус после запуска
            setTimeout(async () => {
                try {
                    const updatedAccount = await connection.getAccountInfo(GAME_SESSION);
                    const newStatus = updatedAccount.data[16];
                    console.log('Новый статус раунда:', statusMap[newStatus] || 'Неизвестный статус');
                } catch (err) {
                    console.error('Ошибка при проверке нового статуса:', err);
                }
            }, 2000); // Ждем 2 секунды для обновления данных
        }

    } catch (error) {
        console.error('Ошибка при запуске нового раунда:', error);
        if (error.logs) {
            console.error('Логи транзакции:', error.logs);
        }
    }
}

// Запускаем скрипт
startNewRound();