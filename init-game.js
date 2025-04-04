const { Connection, PublicKey, Keypair, SystemProgram, SYSVAR_RENT_PUBKEY, 
  Transaction, TransactionInstruction, sendAndConfirmTransaction } = require('@solana/web3.js');
const fs = require('fs');

// Конфигурация
const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
const programId = new PublicKey('GZB6nqB9xSC8VKwWajtCu2TotPXz1mZCR5VwMLEKDj81');
const rewardTokenMint = new PublicKey('4QhhRXq8NvyjpxLiC44nEFzKX9ZRYr9twMvBr7Jm7cLb');

// Загрузка кошелька из файла ключа
const keypairData = Uint8Array.from(JSON.parse(fs.readFileSync(process.env.HOME + '/.config/solana/id.json')));
const signer = Keypair.fromSecretKey(keypairData);

// Дискриминатор для initialize_game_session [127, 189, 104, 88, 218, 56, 57, 243]
const discriminator = Buffer.from([127, 189, 104, 88, 218, 56, 57, 243]);

async function initGameSession() {
  try {
    // Находим PDA для game_session
    const [gameSessionPda] = await PublicKey.findProgramAddress(
      [Buffer.from('game_session')],
      programId
    );
    
    console.log('Game Session PDA:', gameSessionPda.toBase58());
    
    // Создаем инструкцию
    const instruction = new TransactionInstruction({
      keys: [
        { pubkey: signer.publicKey, isSigner: true, isWritable: true },
        { pubkey: rewardTokenMint, isSigner: false, isWritable: false },
        { pubkey: gameSessionPda, isSigner: false, isWritable: true },
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
        { pubkey: SYSVAR_RENT_PUBKEY, isSigner: false, isWritable: false },
      ],
      programId,
      data: discriminator
    });
    
    // Создаем и отправляем транзакцию
    const transaction = new Transaction().add(instruction);
    const signature = await sendAndConfirmTransaction(
      connection,
      transaction,
      [signer]
    );
    
    console.log('Транзакция выполнена успешно:', signature);
    console.log('Игровая сессия инициализирована');
    console.log(`Ссылка на транзакцию: https://explorer.solana.com/tx/${signature}?cluster=devnet`);
    
  } catch (error) {
    console.error('Ошибка при инициализации игровой сессии:', error);
    if (error.logs) {
      console.error('Логи ошибки:', error.logs);
    }
  }
}

initGameSession();