 
const BN = require('bn.js');

// Константы программы
const GAME_SESSION = new PublicKey('DG44Ra5wN6CRWeeBqCWH7V6nfkxK4g6Chwd4f4pT4HCm');

async function checkRoundInfo() {
    try {
        // Подключаемся к сети
        const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
        console.log('Подключение установлено');

        // Получаем информацию об аккаунте игровой сессии
        const gameSessionAccount = await connection.getAccountInfo(GAME_SESSION);
        if (!gameSessionAccount) {
            throw new Error('Аккаунт игровой сессии не найден');
        }

        const data = gameSessionAccount.data;
        
        // Извлекаем данные из аккаунта
        const currentRound = new BN(data.slice(8, 16), 'le').toNumber();
        const roundStatus = data[16];
        
        // Статусы раунда
        const statusMap = {
            0: 'NotStarted',
            1: 'AcceptingBets',
            2: 'BetsClosed',
            3: 'WaitingForRandom',
            4: 'Completed'
        };
        
        console.log('Информация о текущем раунде:');
        console.log('---------------------------');
        console.log('Номер раунда:', currentRound);
        console.log('Статус раунда:', statusMap[roundStatus] || `Неизвестный статус (${roundStatus})`);
        
        // Выводим сырые байты аккаунта для анализа
        console.log('\nСырые данные аккаунта:');
        console.log('Первые 32 байта:', Array.from(data.slice(0, 32)));
        
        // Если статус не AcceptingBets, выводим предупреждение
        if (roundStatus !== 1) {
            console.log('\nВНИМАНИЕ: Текущий раунд НЕ принимает ставки!');
            console.log('Запустите новый раунд с помощью скрипта start-new-round.js');
        } else {
            console.log('\nРаунд готов принимать ставки.');
        }
        
    } catch (error) {
        console.error('Ошибка при проверке информации о раунде:', error);
    }
}

// Запускаем скрипт
checkRoundInfo();