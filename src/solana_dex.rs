use futures::future::join_all;
use futures::StreamExt;
use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
use solana_client::{
    nonblocking::pubsub_client::PubsubClient, rpc_config::RpcAccountInfoConfig,
    rpc_response::Response,
};
use solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use crate::strategy::StreamMessage;

pub struct SolanaDexListener {
    tx: mpsc::Sender<StreamMessage>,
    pubsub_client: Arc<PubsubClient>,
}

#[derive(Debug)]
pub struct RaydiumPool {
    pub token0_amount: f64,
    pub token0_addr: Pubkey,
    pub token1_amount: f64,
    pub token1_addr: Pubkey,
    pub last_modified_slot: u64,
}

fn parse_message(response: Response<UiAccount>) -> Option<(u64, f64)> {
    let slot = response.context.slot;
    if let UiAccountData::Json(parsed) = response.value.data {
        let amount = parsed.parsed["info"]["tokenAmount"]["amount"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap();

        Some((slot, amount))
    } else {
        return None;
    }
}
impl SolanaDexListener {
    pub async fn new(solana_url: String, sender: mpsc::Sender<StreamMessage>) -> Self {
        let pubsub_client = Arc::new(PubsubClient::new(&solana_url).await.unwrap());
        Self {
            pubsub_client,
            tx: sender,
        }
    }

    pub async fn subscribe_and_listen(self: Arc<Self>, vault_addrs: Vec<Pubkey>) {
        let mut handles = Vec::new();

        let mut _ray_pool = Arc::new(Mutex::new(RaydiumPool {
            token0_amount: 0.0,
            token0_addr: vault_addrs[0].clone(),
            token1_amount: 0.0,
            token1_addr: vault_addrs[1].clone(),
            last_modified_slot: 0,
        }));

        for addr in vault_addrs {
            let tx_clone = self.tx.clone();
            let ray_pool_cloned = _ray_pool.clone();
            let client = Arc::clone(&self.pubsub_client);
            handles.push(tokio::spawn(async move {
                let (mut stream, _) = client
                    .account_subscribe(
                        &addr,
                        Some(RpcAccountInfoConfig {
                            encoding: Some(UiAccountEncoding::JsonParsed),
                            data_slice: None,
                            commitment: Some(CommitmentConfig::confirmed()),
                            min_context_slot: None,
                        }),
                    )
                    .await
                    .unwrap();

                while let Some(response) = stream.next().await {
                    let (slot, amount) = parse_message(response.clone()).unwrap();
                    let mut ray_pool = ray_pool_cloned.lock().await;
                    if ray_pool.token0_addr == addr {
                        ray_pool.token0_amount = amount;
                        if ray_pool.last_modified_slot != slot {
                            ray_pool.last_modified_slot = slot;
                        } else {
                            let dex_price = ray_pool.token1_amount / ray_pool.token0_amount;
                            tx_clone
                                .send(StreamMessage::DexPrice(dex_price))
                                .await
                                .unwrap();
                        }
                    } else if ray_pool.token1_addr == addr {
                        ray_pool.token1_amount = amount;
                        if ray_pool.last_modified_slot != slot {
                            ray_pool.last_modified_slot = slot;
                        } else {
                            let dex_price = ray_pool.token1_amount / ray_pool.token0_amount;
                            tx_clone
                                .send(StreamMessage::DexPrice(dex_price))
                                .await
                                .unwrap();
                        }
                    }
                }
            }));
        }
        join_all(handles).await;
    }
}
#[cfg(test)]
mod test_sol {
    use super::*;
    #[tokio::test]
    async fn test_sol() {
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            let solana_url =
                "wss://mainnet.helius-rpc.com/?api-key=115eda22-77db-42e6-916c-9f53b21a5538";

            let vault1_addr: Pubkey = "4Vc6N76UBu26c3jJDKBAbvSD7zPLuQWStBk7QgVEoeoS" //token0
                .parse()
                .unwrap();
            let vault2_addr: Pubkey = "n6CwMY77wdEftf2VF6uPvbusYoraYUci3nYBPqH1DJ5" //token1
                .parse()
                .unwrap();
            let vault_addrs = vec![vault1_addr, vault2_addr]; //Raydium vault addresses for token0 and token1
            let sol_client =
                Arc::new(SolanaDexListener::new(solana_url.to_string(), tx.clone()).await);

            sol_client.subscribe_and_listen(vault_addrs.clone()).await;
        });
    }
}
