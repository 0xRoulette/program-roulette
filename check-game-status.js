const { Connection, PublicKey } = require('@solana/web3.js');
const BN = require('bn.js');

// НОВЫЙ ID программы
const PROGRAM_ID = new PublicKey('GZB6nqB9xSC8VKwWajtCu2TotPXz1mZCR5VwMLEKDj81');

async function checkGameStatus() {
  try {
    // Подключаемся к сети
    const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
    console.log('Подключение установлено');

    // Получаем PDA для game_session относительно НОВОЙ программы
    const [gameSessionPda] = await PublicKey.findProgramAddress(
      [Buffer.from('game_session')],
      PROGRAM_ID
    );

    console.log('PDA игровой сессии для новой программы:', gameSessionPda.toString());

    // Проверяем, существует ли аккаунт на этом адресе
    const accountInfo = await connection.getAccountInfo(gameSessionPda);

    if (!accountInfo) {
      console.log('❌ Игра НЕ инициализирована на новой программе. Нужно запустить init-game.js');
      return;
    }

    console.log('✅ Аккаунт игровой сессии существует');
    console.log('Данные аккаунта получены, размер:', accountInfo.data.length);

    // Извлекаем данные из аккаунта
    const accountData = accountInfo.data;
    const currentRound = new BN(accountData.slice(8, 16), 'le').toNumber();
    const roundStartTime = new BN(accountData.slice(16, 24), 'le').toNumber();
    const roundStatus = accountData[24];
    
    const statusMap = {
      0: 'NotStarted',
      1: 'AcceptingBets',
      2: 'BetsClosed',
      3: 'WaitingForRandom',
      4: 'Completed'
    };

    // Выводим информацию о раунде
    console.log('Текущий раунд:', currentRound);
    console.log('Время начала раунда:', new Date(roundStartTime * 1000).toLocaleString());
    console.log('Статус раунда:', statusMap[roundStatus] || `Неизвестный (${roundStatus})`);
    
    if (roundStatus === 1) {
      console.log('✅ Ставки принимаются! Можно отправлять транзакцию.');
    } else {
      console.log('❌ Раунд не принимает ставки в данный момент.');
    }
    
  } catch (error) {
    console.error('Ошибка при получении статуса игры:', error);
  }
}

checkGameStatus();