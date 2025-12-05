# 🔁 Polymarket Copy Trading Bot

Polymarket copy trading bot is an automated trading system designed to replicate the trading strategies of successful Polymarket traders in real-time. A straightforward, high-speed copy trading bot for **Polymarket**, built with **TypeScript** for type safety and modern JavaScript features.

<div align="center">
  <a href="../../releases/latest">
    <img width="1200" alt="Polymarket copy trading bot is an automated trading system designed to replicate the trading strategies of successful Polymarket traders in real-time." src="assets/polymarket.png" />
  </a>
</div>

---

## Overview

This bot monitors selected Polymarket wallets in real time and **automatically mirrors their trades**.  

Perfect for users who want a **simple, fast, and reliable** copy-trading setup with the benefits of TypeScript's type safety and modern tooling.

---
```
polymarket-copy-bot/
│
├── src/
│   ├── main.ts
│   ├── config.ts
│   ├── api.ts
│   ├── polymarket_api.ts
│   ├── watcher.ts
│   ├── interpreter.ts
│   ├── sizing.ts
│   ├── executor.ts
│   ├── risk.ts
│   ├── strategy.ts
│   └── agent.ts
│
├── package.json
├── tsconfig.json
└── .env
```
---
## Core Features

- **Auto Copy Trading** — automatically replicates trades from a target Polymarket trader.  
- **Risk Controls** — adjustable fetch intervals, retry limits, and timestamp filtering.  
- **Type Safety** — full TypeScript support with compile-time type checking.  
- **Simple Configuration** — all settings managed through a `.env` file and `config.ts`.

### Real-Time Wallet Mirroring

> The bot continuously monitors target wallets: detects trades instantly, identifies the event, direction, and stake, executes mirrored trades within <150ms, and uses asynchronous processing for maximum speed.


---

### 2. Position Scaling

Choose how the bot sizes your trades:

**Proportional Mode:**
```

YourStake = (YourBank / TraderBank) × TraderStake

```

**Fixed Allocation Mode:**
```

Always bet $X per trade

```

Includes:
- Min/max trade size  
- Liquidity checks  
- Event-level exposure limits  

---

### 3. Multi-Wallet Copying

Copy multiple wallets at once.  
You can set:

- Equal weights  
- Custom weights  
- Per-wallet limits  

The bot will open/close positions in sync with each tracked wallet.

---

### 4. Fail-Safe Execution Layer

Built for reliability:

- No duplicate trades  
- Automatic retries  
- Handles Polymarket API interruptions  
- Auto-exit when the original wallet exits the position  

---

## ⚙️ Architecture (Simplified)

```

+------------------------------+
|      Wallet Watcher          |
|  Real-time wallet tracking   |
+--------------+---------------+
|
v
+------------------------------+
|     Trade Interpreter        |
| Detect event, direction, $   |
+--------------+---------------+
|
v
+------------------------------+
|     Position Sizing Engine   |
| Scale or fixed allocation    |
+--------------+---------------+
|
v
+------------------------------+
|       Trade Executor         |
| Market/limit execution       |
+--------------+---------------+
|
v
Polymarket API

```

---
## Example Copy Script

1. A tracked wallet buys **"Biden wins MI"** at 48%.
2. Bot detects the trade instantly.
3. Your sizing rule is applied (e.g., $25 fixed).
4. The bot mirrors the trade within ~150 ms.
5. When the wallet exits, your position closes automatically.
6. Optional rule: the bot can automatically close your trade once it reaches **X% profit**, regardless of what the tracked wallet does.

---

## Risk Controls

- Max stake per event  
- Daily/weekly exposure limits  
- Liquidity filters  
- Per-wallet exposure caps  
- Automatic recovery  
- Full trade log  

---
## 🖥️ Installation and Launch

### Prerequisites

- **Node.js 18+** - Download from [nodejs.org](https://nodejs.org/)
- **npm** (comes with Node.js)

### Installation Steps

1. **Clone the repository**
   ```bash
   git clone <repository-url>
   cd polymarket-copy-trading-bot
   ```

2. **Install dependencies**
   ```bash
   npm install
   ```

3. **Set up environment variables**
   - Create a `.env` file in the root directory
   - Add your configuration:
     ```env
     POLYMARKET_API=https://api.polymarket.com
     OPENAI_API_KEY=your_openai_api_key_here
     ```

4. **Configure wallets**
   - Edit `src/config.ts` and update the `wallets_to_track` array with actual wallet addresses

5. **Run the bot**
   ```bash
   # Development mode
   npm run dev
   
   # Production mode
   npm run build
   npm start
   ```

### Available Scripts

- `npm run dev` - Run in development mode with tsx
- `npm run build` - Compile TypeScript to JavaScript
- `npm start` - Run the compiled JavaScript
- `npm run watch` - Watch mode (auto-rebuild on changes)
- `npm run cli` - Run CLI commands (see CLI_USAGE.md)

### CLI Commands

The bot includes a user-friendly CLI. Quick start:

```bash
# Initialize
npm run cli init

# Add wallets
npm run cli wallets add <address>

# Configure
npm run cli config --mode fixed --stake 25

# Start bot
npm run cli start
```

See [CLI_USAGE.md](CLI_USAGE.md) for complete CLI documentation.

---
## Tech Stack

| Layer     | Technology          |
|-----------|----------------------|
| Language  | TypeScript 5.3+      |
| Runtime   | Node.js 18+          |
| HTTP      | axios                |
| Data      | Polymarket API       |
| AI        | OpenAI API           |
| Interface | CLI                  |

---

## Development

### Project Structure

- `src/main.ts` - Main entry point
- `src/config.ts` - Configuration management
- `src/api.ts` - Polymarket API client
- `src/watcher.ts` - Wallet monitoring with async generators
- `src/interpreter.ts` - Trade normalization
- `src/sizing.ts` - Position sizing logic
- `src/executor.ts` - Trade execution
- `src/risk.ts` - Risk management
- `src/strategy.ts` - Trading strategies (Kelly criterion, expected value)
- `src/agent.ts` - AI event analysis with OpenAI

### Type Checking

```bash
npx tsc --noEmit
```

### Additional Documentation

- [QUICKSTART.md](QUICKSTART.md) - Quick start guide
- [README_TYPESCRIPT.md](README_TYPESCRIPT.md) - Detailed TypeScript documentation
- [MIGRATION_SUMMARY.md](MIGRATION_SUMMARY.md) - Python to TypeScript migration details

---

## Contact

For questions, support, or collaboration, reach out on Telegram: [@terauss](https://t.me/terauss)

---

## License

MIT License.

---
`Copy trades fast. Copy trades directly. No extra logic — just pure mirroring`
