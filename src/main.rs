use gate::GatePublicWebsocketClient;
use solana_dex::SolanaDexListener;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use strategy::Strategy;

use tokio::sync::mpsc;
mod gate;
mod solana_dex;
mod strategy;

#[tokio::main]
async fn main() {
    let solana_url = "wss://mainnet.helius-rpc.com/?api-key=115eda22-77db-42e6-916c-9f53b21a5538";
    let vault1_addr: Pubkey = "4Vc6N76UBu26c3jJDKBAbvSD7zPLuQWStBk7QgVEoeoS" //token0
        .parse()
        .unwrap();
    let vault2_addr: Pubkey = "n6CwMY77wdEftf2VF6uPvbusYoraYUci3nYBPqH1DJ5" //token1
        .parse()
        .unwrap();
    let vault_addrs = vec![vault1_addr, vault2_addr]; //Raydium vault addresses for token0 and token1

    let (tx, rx) = mpsc::channel(100);
    let cloned_tx = tx.clone();
    tokio::spawn(async move {
        let gate_client = GatePublicWebsocketClient::new(cloned_tx);
        gate_client.connect_and_subscribe().await;
    });
    tokio::spawn(async move {
        let sol_client = Arc::new(SolanaDexListener::new(solana_url.to_string(), tx.clone()).await);
        sol_client.subscribe_and_listen(vault_addrs.clone()).await;
    });
    let strategy = Strategy::new();

    strategy.run(rx).await;
}
