const { Connection, PublicKey, Keypair } = require('@solana/web3.js');
const { getAssociatedTokenAddress } = require('@solana/spl-token');
const fs = require('fs');

// Константы программы и токенов
const TOKEN_MINT = new PublicKey('G6tyrxS8yA18zPV5QZoYUE4igw7BSvDqqWETNVuGS83X');

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

async function checkTokenBalance() {
  try {
    // Загружаем кошелек
    const wallet = loadWalletKey(process.env.HOME + '/.config/solana/id.json');
    console.log('Кошелек загружен:', wallet.publicKey.toString());
    
    // Подключаемся к сети
    const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
    console.log('Подключение установлено');
    
    // Получаем адрес токен-аккаунта для этого минта и кошелька
    const associatedTokenAddress = await getAssociatedTokenAddress(
      TOKEN_MINT,
      wallet.publicKey
    );
    
    console.log('Адрес ассоциированного токен-аккаунта:', associatedTokenAddress.toString());
    
    // Получаем данные аккаунта
    const accountInfo = await connection.getAccountInfo(associatedTokenAddress);
    
    if (!accountInfo) {
      console.log('Токен-аккаунт не существует! Запустите create-token-account.js');
      return;
    }
    
    // Получаем баланс через запрос информации о токен-аккаунте
    const tokenAccountInfo = await connection.getParsedAccountInfo(associatedTokenAddress);
    
    if (tokenAccountInfo?.value?.data && 'parsed' in tokenAccountInfo.value.data) {
      const tokenInfo = tokenAccountInfo.value.data.parsed;
      console.log('Информация о токен-аккаунте:', tokenInfo);
      
      if (tokenInfo.info && tokenInfo.info.tokenAmount) {
        const amount = tokenInfo.info.tokenAmount;
        console.log(`Баланс: ${amount.uiAmount} токенов (${amount.amount} с учетом десятичных знаков)`);
      }
    } else {
      console.log('Не удалось получить информацию о балансе');
    }
    
  } catch (error) {
    console.error('Ошибка при проверке баланса:', error);
  }
}

// Запускаем скрипт
checkTokenBalance();