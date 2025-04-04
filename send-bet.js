const { Connection, PublicKey, Keypair, Transaction, TransactionInstruction, SystemProgram, SYSVAR_RENT_PUBKEY } = require('@solana/web3.js');
const { TOKEN_PROGRAM_ID, getAssociatedTokenAddress } = require('@solana/spl-token');
const BN = require('bn.js');
const fs = require('fs');

// Константы программы и токенов
const PROGRAM_ID = new PublicKey('DM3usAt52jxzrisiGw1n1fsFVD6Nwv1TZRM5uy79YZgs');
const GAME_SESSION = new PublicKey('DG44Ra5wN6CRWeeBqCWH7V6nfkxK4g6Chwd4f4pT4HCm');
const TOKEN_MINT = new PublicKey('G6tyrxS8yA18zPV5QZoYUE4igw7BSvDqqWETNVuGS83X');
const VAULT_ADDRESS = new PublicKey('8SyiPHDrKfi85cuJtyPKHLh8PfWr9TaQzfjQjdLBLWrg');
const VAULT_TOKEN_ACCOUNT = new PublicKey('BbEZ48kngctxJFu5VG7HTcSy2FSoTM1JSph2hNGeoX7k');

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

async function sendBet() {
    try {
        // Загружаем кошелек
        const wallet = loadWalletKey(process.env.HOME + '/.config/solana/id.json');
        console.log('Кошелек загружен:', wallet.publicKey.toString());

        // Подключаемся к сети
        const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
        console.log('Подключение установлено');

        // Получаем токен-аккаунт плеера
        const playerTokenAccount = await getAssociatedTokenAddress(
            TOKEN_MINT,
            wallet.publicKey
        );
        console.log('Токен-аккаунт игрока:', playerTokenAccount.toString());

        // Проверяем баланс токенов
        try {
            const tokenInfo = await connection.getParsedAccountInfo(playerTokenAccount);
            
            if (tokenInfo.value) {
                const tokenBalance = tokenInfo.value.data.parsed.info.tokenAmount;
                console.log(`Баланс токенов: ${tokenBalance.uiAmount} (${tokenBalance.amount} единиц)`);
                console.log(`Decimal Places: ${tokenBalance.decimals}`);
            } else {
                console.log('Токен-аккаунт не найден');
            }
        } catch (err) {
            console.warn('Не удалось получить информацию о балансе:', err);
        }

        // Получаем текущий раунд из GameSession
        const gameSessionAccount = await connection.getAccountInfo(GAME_SESSION);
        if (!gameSessionAccount) {
            throw new Error('Аккаунт игровой сессии не найден');
        }

        // Извлекаем текущий раунд (u64, 8 байт после дискриминатора)
        const accountData = gameSessionAccount.data;
        const currentRound = new BN(accountData.slice(8, 16), 'le').toNumber();
        console.log('Текущий раунд:', currentRound);

        // Проверка статуса раунда (теперь мы знаем, что статус на позиции 24)
        const roundStatus = accountData[24];
        const statusMap = {
            0: 'NotStarted',
            1: 'AcceptingBets',
            2: 'BetsClosed',
            3: 'WaitingForRandom',
            4: 'Completed'
        };
        console.log('Статус раунда:', statusMap[roundStatus] || `Неизвестный (${roundStatus})`);

        if (roundStatus !== 1) {
            throw new Error(`Раунд не принимает ставки! Статус: ${statusMap[roundStatus] || roundStatus}`);
        }

        // Создаем PDA для player_bets
        const [playerBetsPDA, playerBetsBump] = await PublicKey.findProgramAddress(
            [
                Buffer.from('player_bets'),
                GAME_SESSION.toBuffer(),
                wallet.publicKey.toBuffer(),
                Buffer.from(new BN(currentRound).toArray('le', 8))
            ],
            PROGRAM_ID
        );

        console.log('Player Bets PDA:', playerBetsPDA.toString());

        // Дискриминатор для place_bet_simple из IDL
        const discriminator = Buffer.from([54, 83, 70, 219, 157, 29, 87, 132]);

        // ВАЖНО: Используем минимальную ставку, которая точно пройдет
        // Можно постепенно увеличивать до нужного размера
        const betAmount = new BN(10_000); // Небольшая сумма (проверено - работает)
        console.log('Размер ставки:', betAmount.toString(), 'единиц');
        
        // Тип ставки и числа
        const betType = 8; // Even (Четное)
        const numbers = [0, 0, 0, 0]; // Пустые числа для типа ставки Even

        // Сериализуем данные
        const amountBuffer = betAmount.toArrayLike(Buffer, 'le', 8);
        const betTypeBuffer = Buffer.from([betType]);
        const numbersBuffer = Buffer.alloc(4);
        for (let i = 0; i < numbers.length && i < 4; i++) {
            numbersBuffer[i] = numbers[i];
        }

        // Формируем данные инструкции
        const instructionData = Buffer.concat([
            discriminator,
            amountBuffer,
            betTypeBuffer,
            numbersBuffer
        ]);

        console.log('Данные инструкции готовы');

        // Создаем инструкцию
        const instruction = new TransactionInstruction({
            keys: [
                { pubkey: VAULT_ADDRESS, isSigner: false, isWritable: true },
                { pubkey: GAME_SESSION, isSigner: false, isWritable: true },
                { pubkey: playerTokenAccount, isSigner: false, isWritable: true },
                { pubkey: VAULT_TOKEN_ACCOUNT, isSigner: false, isWritable: true },
                { pubkey: wallet.publicKey, isSigner: true, isWritable: true },
                { pubkey: playerBetsPDA, isSigner: false, isWritable: true },
                { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
                { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
                { pubkey: SYSVAR_RENT_PUBKEY, isSigner: false, isWritable: false },
            ],
            programId: PROGRAM_ID,
            data: instructionData
        });

        // Создаем и отправляем транзакцию
        const transaction = new Transaction().add(instruction);
        transaction.feePayer = wallet.publicKey;

        // Получаем последний блокхеш
        const blockHash = await connection.getLatestBlockhash();
        transaction.recentBlockhash = blockHash.blockhash;

        // Подписываем транзакцию
        transaction.sign(wallet);

        // Отправляем транзакцию
        console.log('Отправка транзакции со ставкой...');
        const txid = await connection.sendRawTransaction(
            transaction.serialize(),
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
            console.log('Ставка успешно размещена!');
        }

    } catch (error) {
        console.error('Ошибка при отправке ставки:', error);
        if (error.logs) {
            console.error('Логи транзакции:', error.logs);
        }
    }
}

// Запускаем скрипт
sendBet();