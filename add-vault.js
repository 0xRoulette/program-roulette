const { Connection, PublicKey, Keypair, Transaction, TransactionInstruction, sendAndConfirmTransaction, SystemProgram
} = require('@solana/web3.js');
const { TOKEN_PROGRAM_ID, getAssociatedTokenAddress
} = require('@solana/spl-token');
const fs = require('fs');


const RENT_PROGRAM_ID = new PublicKey('SysvarRent111111111111111111111111111111111');


async function provideLiquidity() {
  try {
    // Настройки подключения
    const connection = new Connection('https: //api.devnet.solana.com', 'confirmed');
    const programId = new PublicKey('GZB6nqB9xSC8VKwWajtCu2TotPXz1mZCR5VwMLEKDj81');
    const rewardTokenMint = new PublicKey('4QhhRXq8NvyjpxLiC44nEFzKX9ZRYr9twMvBr7Jm7cLb');
    
    // Загрузка кошелька
    const keypairData = Uint8Array.from(JSON.parse(fs.readFileSync(process.env.HOME + '/.config/solana/id.json')));
    const signer = Keypair.fromSecretKey(keypairData);
    console.log('Кошелек загружен:', signer.publicKey.toString());
    
    // Получение PDA для хранилища
    const [vaultPda
    ] = await PublicKey.findProgramAddress(
      [Buffer.from('vault'), rewardTokenMint.toBuffer()
    ],
      programId
    );
    console.log('Адрес хранилища:', vaultPda.toString());
    
    // Получение токен-аккаунта пользователя
    const providerTokenAccount = await getAssociatedTokenAddress(
      rewardTokenMint,
      signer.publicKey
    );
    console.log('Токен-аккаунт пользователя:', providerTokenAccount.toString());
    
    // Получение токен-аккаунта хранилища
    const vaultAccount = await connection.getAccountInfo(vaultPda);
    let vaultTokenAccount;
    
    if (vaultAccount) {
      // Если хранилище уже существует, получаем его токен-аккаунт
      vaultTokenAccount = new PublicKey(vaultAccount.data.slice(32,
      64));
      console.log('Токен-аккаунт хранилища:', vaultTokenAccount.toString());
      
      // Дискриминатор для provide_liquidity
      const discriminator = Buffer.from([
        226,
        22,
        158,
        55,
        239,
        119,
        220,
        187
      ]);
      
      // Создаем буфер для аргумента amount (u64)
      const amount = 100_000_000; // 100 токенов
      const amountBuffer = Buffer.alloc(8);
      amountBuffer.writeBigUInt64LE(BigInt(amount));
      
      // Объединяем дискриминатор и аргумент
      const data = Buffer.concat([discriminator, amountBuffer
      ]);
      
      // Создание инструкции
      const instruction = new TransactionInstruction({
        keys: [
          { pubkey: vaultPda, isSigner: false, isWritable: true
          },
          { pubkey: providerTokenAccount, isSigner: false, isWritable: true
          },
          { pubkey: vaultTokenAccount, isSigner: false, isWritable: true
          },
          { pubkey: signer.publicKey, isSigner: true, isWritable: true
          },
          { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false
          },
          { pubkey: SystemProgram.programId, isSigner: false, isWritable: false
          },
        ],
        programId,
        data,
      });
      
      // Отправка транзакции
      const tx = new Transaction().add(instruction);
      const signature = await sendAndConfirmTransaction(connection, tx,
      [signer
      ]);
      
      console.log('Транзакция выполнена успешно:', signature);
      console.log(`Ссылка на транзакцию: https: //explorer.solana.com/tx/${signature}?cluster=devnet`);
    } else {
      console.log('Хранилище не существует, используйте initialize_and_provide_liquidity');
      // Код для initialize_and_provide_liquidity
      // Создание токен-аккаунта для хранилища (это PDA)
      const [vaultTokenAccount
      ] = await PublicKey.findProgramAddress(
        [Buffer.from('token_account'), vaultPda.toBuffer()
      ],
        programId
      );
      console.log('Создаваемый токен-аккаунт хранилища:', vaultTokenAccount.toString());
      
      // Дискриминатор для initialize_and_provide_liquidity
      const discriminator = Buffer.from([
        27,
        28,
        233,
        66,
        11,
        249,
        201,
        167
      ]);
      
      // Создаем буфер для аргумента amount (u64)
      const amount = 100_000_000; // 100 токенов
      const amountBuffer = Buffer.alloc(8);
      amountBuffer.writeBigUInt64LE(BigInt(amount));
      
      // Объединяем дискриминатор и аргумент
      const data = Buffer.concat([discriminator, amountBuffer
      ]);
      
      // Создание инструкции
      const instruction = new TransactionInstruction({
        keys: [
          { pubkey: signer.publicKey, isSigner: true, isWritable: true
          },
          { pubkey: rewardTokenMint, isSigner: false, isWritable: false
          },
          { pubkey: vaultPda, isSigner: false, isWritable: true
          },
          { pubkey: providerTokenAccount, isSigner: false, isWritable: true
          },
          { pubkey: vaultTokenAccount, isSigner: false, isWritable: true
          },
          { pubkey: signer.publicKey, isSigner: true, isWritable: true
          },
          { pubkey: SystemProgram.programId, isSigner: false, isWritable: false
          },
          { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false
          },
          { pubkey: RENT_PROGRAM_ID, isSigner: false, isWritable: false
          },
        ],
        programId,
        data,
      });
      
      // Отправка транзакции
      const tx = new Transaction().add(instruction);
      const signature = await sendAndConfirmTransaction(connection, tx,
      [signer
      ]);
      
      console.log('Транзакция инициализации выполнена успешно:', signature);
      console.log(`Ссылка на транзакцию: https: //explorer.solana.com/tx/${signature}?cluster=devnet`);
    }
  } catch (error) {
    console.error('Ошибка при пополнении хранилища:', error);
    if (error.logs) {
      console.error('Логи ошибки:', error.logs);
    }
  }
}

provideLiquidity();