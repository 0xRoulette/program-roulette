 

// Константы программы
const PROGRAM_ID = new PublicKey('DM3usAt52jxzrisiGw1n1fsFVD6Nwv1TZRM5uy79YZgs');
const GAME_SESSION = new PublicKey('DG44Ra5wN6CRWeeBqCWH7V6nfkxK4g6Chwd4f4pT4HCm');
const VAULT_ADDRESS = new PublicKey('8SyiPHDrKfi85cuJtyPKHLh8PfWr9TaQzfjQjdLBLWrg');

async function analyzeContract() {
    try {
        // Подключаемся к сети
        const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
        console.log('Подключение установлено');

        // Получаем информацию о программе
        const programInfo = await connection.getAccountInfo(PROGRAM_ID);
        console.log(`Размер программы: ${programInfo.data.length} байт`);
        console.log(`Владелец программы: ${programInfo.owner.toString()}`);

        // Получаем информацию о хранилище
        const vaultInfo = await connection.getAccountInfo(VAULT_ADDRESS);
        console.log(`\nИнформация о хранилище:`);
        console.log(`Размер данных: ${vaultInfo.data.length} байт`);
        console.log(`Владелец: ${vaultInfo.owner.toString()}`);

        // Получаем все транзакции программы
        console.log(`\nПоследние транзакции программы:`);
        const signatures = await connection.getSignaturesForAddress(PROGRAM_ID, { limit: 5 });
        
        for (const sig of signatures) {
            console.log(`  Сигнатура: ${sig.signature}`);
            console.log(`  Статус: ${sig.err ? 'Ошибка' : 'Успех'}`);
            console.log(`  Слот: ${sig.slot}`);
            console.log(`  Подтверждена: ${sig.confirmationStatus}`);
            console.log('  ------');
        }

    } catch (error) {
        console.error('Ошибка при анализе контракта:', error);
    }
}

// Запускаем скрипт
analyzeContract();