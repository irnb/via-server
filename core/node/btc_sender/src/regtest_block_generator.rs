use std::time::Duration;

use anyhow::Context;
use bitcoincore_rpc::{
    bitcoin::{Address, Network},
    Auth, Client, RpcApi,
};
use tokio::sync::watch;

const BTC_WALLET_NAME: &str = "regtest_wallet";

pub struct BitcoinBlockGenerator {
    client: Client,
    config: BlockGeneratorConfig,
    mining_address: Address,
}

// TODO: change types
#[derive(Clone)]
pub struct BlockGeneratorConfig {
    pub block_time: Duration,
    pub bitcoin_rpc_url: String,
    pub bitcoin_rpc_user: String,
    pub bitcoin_rpc_pass: String,
}

impl BitcoinBlockGenerator {
    pub fn new(config: BlockGeneratorConfig) -> anyhow::Result<Self> {
        let auth = Auth::UserPass(
            config.bitcoin_rpc_user.clone(),
            config.bitcoin_rpc_pass.clone(),
        );
        let client = Client::new(&config.bitcoin_rpc_url, auth)?;

        Self::load_or_create_wallet(&client)?;

        let mining_address = client
            .get_new_address(None, None)?
            .require_network(Network::Regtest)?;

        tracing::info!(target: "bitcoin_block_generator", "Mining address: {}", mining_address);

        Ok(Self {
            client,
            config,
            mining_address,
        })
    }

    fn load_or_create_wallet(client: &Client) -> anyhow::Result<()> {
        tracing::info!(target: "bitcoin_block_generator", "Loading or creating wallet: {}", BTC_WALLET_NAME);

        match client.list_wallets() {
            Ok(wallets) => {
                if wallets.is_empty() {
                    client.create_wallet(BTC_WALLET_NAME, None, None, None, None)?;
                    tracing::info!(target: "bitcoin_block_generator", "Created new wallet: {}", BTC_WALLET_NAME);
                } else {
                    let wallet_name = &wallets[0];
                    client.load_wallet(wallet_name)?;
                    tracing::info!(target: "bitcoin_block_generator", "Loaded existing wallet: {}", BTC_WALLET_NAME);
                }
            }
            Err(e) => {
                tracing::warn!(target: "bitcoin_block_generator", "Could not list wallets: {}. Attempting to create a new one.", e);
                client.create_wallet(BTC_WALLET_NAME, None, None, None, None)?;
                tracing::info!(target: "bitcoin_block_generator", "Created new wallet: {}", BTC_WALLET_NAME);
            }
        }
        Ok(())
    }

    async fn generate_block(&self) -> anyhow::Result<String> {
        let block_hash = self.client.generate_to_address(1, &self.mining_address)?;
        Ok(block_hash[0].to_string())
    }

    async fn generate_initial_blocks(&self) -> anyhow::Result<()> {
        tracing::info!(target: "bitcoin_block_generator", "Generating initial 100 blocks...");
        let block_hashes = self
            .client
            .generate_to_address(100, &self.mining_address)
            .context("Failed to generate initial blocks")?;
        tracing::info!(target: "bitcoin_block_generator", "Generated 100 blocks. Last block hash: {}", block_hashes.last().unwrap());
        Ok(())
    }

    async fn check_balance(&self) -> anyhow::Result<f64> {
        let balance = self
            .client
            .get_received_by_address(&self.mining_address, None)?;
        let balance_btc = balance.to_btc();
        tracing::info!(target: "bitcoin_block_generator", "Current balance: {} BTC", balance_btc);
        Ok(balance_btc)
    }

    pub async fn run(self, stop_receiver: watch::Receiver<bool>) -> anyhow::Result<()> {
        self.check_balance().await?;

        loop {
            if *stop_receiver.borrow() {
                tracing::info!(target: "bitcoin_block_generator", "Stop signal received, BitcoinBlockGenerator is shutting down");
                break;
            }

            match self.generate_block().await {
                Ok(block_hash) => {
                    tracing::info!(target: "bitcoin_block_generator", "Generated new block: {}", block_hash);
                    self.check_balance().await?;
                }
                Err(e) => {
                    tracing::warn!(target: "bitcoin_block_generator", "Failed to generate block: {:?}", e);
                }
            }

            tokio::time::sleep(self.config.block_time).await;
        }

        self.check_balance().await?;

        Ok(())
    }
}
