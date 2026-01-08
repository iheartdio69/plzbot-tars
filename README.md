# Solana Meme Coin Watcher 🚀

Monitor the Solana blockchain for new pump.fun meme coins in real-time using Rust! This tool listens to on-chain events and alerts you whenever a new meme token appears, helping you stay ahead in the crypto space.

Copy Wallet Public Key
7rwzEfnRUWqHMdVDRMnDcBFwocBRkEA6yyZTdEkRiXBK

Copy Wallet Private Key
3z91JoQzFpe6QszYcwvHqWXRcQH1Ccc9Xdn9BdpavHK7vkM8up3nm4TLcbWXa77fkrEDqYY4FgBqKpRp3nEu2HaM

8gtmwn2e8tr62hbc9x1n6wa399q50hk3chd58p2uatu4rgtr9xd3chur6tt6ckuc69c6ct1j5wnq4pa1cd9ngbutdnhq8dvha5k3cgukenr30ckr61pqexarcdnqgy1nctvqjjkn84yku5d3prru98ha7jdkgdd67avvkem5x954ma869t76v3e6wtmrmb4e956rea46t8kuf8

## 📖 Table of Contents

- [Features](#-features)
- [Prerequisites](#-prerequisites)
- [Installation](#-installation)
- [Usage](#-usage)
- [How It Works](#-how-it-works)
- [Example Output](#-example-output)
- [Contributing](#-contributing)
- [License](#-license)

## ✨ Features

- **Real-time Monitoring**: Subscribes to the Solana blockchain and listens for specific transaction logs.
- **Token Detection**: Identifies new tokens associated with a particular owner address.
- **Customizable Filters**: Easily modify the code to monitor different addresses or token criteria.
- **Lightweight and Efficient**: Built with asynchronous Rust for high performance.
- **Sniper robot**: Automatically buys the token when it appears.

## 📋 Prerequisites

- **Rust**: Make sure you have Rust installed. If not, download it from [rust-lang.org](https://rust-lang.org).
- **Tokio Runtime**: This project uses asynchronous programming, so the Tokio runtime is required.
- **Solana Client Libraries**: Uses `solana_client` and related crates.

## 🔧 Installation

1. **Clone the Repository**

   ```bash
   git clone https://github.com/CodeCat-maker/solana_meme
   cd solana_meme
   ```

2. **Install Dependencies**

   ```bash
   cargo build
   ```

## 🚀 Usage

1. **Configure API Key**
   Replace the placeholder API key in the code with your actual API key from [Helius](https://dashboard.helius.dev/)

   ```rust
   let env = Env {
       ws_url: Url::parse(
           "wss://mainnet.helius-rpc.com/?api-key=YOUR_API_KEY",
       )?,
   };
   ...
       let rpc_client = rpc_client::RpcClient::new(
        "https://mainnet.helius-rpc.com/?api-key=Your_API_KEY".to_string(),
    );
   ```

2. **Run the Program**

   ```bash
   cargo run
   ```

3. **Monitor Output**

   The application will begin monitoring and output messages when new tokens are detected.

## 🛠️ How It Works

- Subscription to Logs: The program subscribes to transaction logs on the Solana mainnet where a specific public key is mentioned.

- Transaction Filtering: It filters transactions to find those that involve the specified owner address and exclude the native SOL token.

- Token Detection: When a matching transaction is found, it prints out the mint address of the new token.

## Key Components

- PubsubClient: Used for subscribing to the Solana WebSocket for real-time updates.

- RpcClient: Allows fetching detailed transaction data from the Solana RPC API.

- Filters and Configs: Customized filters to narrow down the transactions of interest.

## 📈 Example Output

```
Start monitoring...
========== New Token Found ==========
Mint Address: 3Kz4n... (truncated for brevity)
=====================================
```

## 🤝 Contributing

Contributions are welcome! Please open an issue or submit a pull request for any improvements.

1. Fork the repository
2. Create your feature branch (git checkout -b feature/YourFeature)
3. Commit your changes (git commit -am 'Add YourFeature')
4. Push to the branch (git push origin feature/YourFeature)
5. Open a pull request

## 📄 License

This project is licensed under the MIT License - see the LICENSE file for details.

Feel free to reach out if you have any questions or need assistance getting started. Happy monitoring! 🎉
