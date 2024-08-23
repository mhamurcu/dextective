use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use solana_account_decoder::{UiAccountData, UiAccountEncoding};
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

use tokio::sync::mpsc;

const MIN_BPS: f64 = 30.0;

#[derive(Debug, Clone)]
enum StreamMessage {
    Solana(Pubkey, f64, u64),
    CexPrice(f64, f64, f64),
}

#[derive(Debug, Default)]
struct RaydiumPool {
    token0_amount: f64,
    token1_amount: f64,
    last_modified_slot: u64,
}
async fn handle_solana_streams(
    vault_addrs: Vec<Pubkey>,
    solana_url: &str,
    tx: mpsc::Sender<StreamMessage>,
) {
    let pubsub_client = Arc::new(PubsubClient::new(solana_url).await.unwrap());
    for addr in vault_addrs {
        let tx_clone = tx.clone();
        let client = Arc::clone(&pubsub_client);
        tokio::spawn(async move {
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
            println!("subscribed");

            while let Some(response) = stream.next().await {
                if let UiAccountData::Json(parsed) = response.value.data {
                    let slot = response.context.slot;
                    let amount = parsed.parsed["info"]["tokenAmount"]["amount"]
                        .as_str()
                        .unwrap()
                        .parse::<f64>()
                        .unwrap();

                    tx_clone
                        .send(StreamMessage::Solana(addr.clone(), amount, slot))
                        .await
                        .unwrap();
                }
            }
        });
    }
}

async fn handle_cex_stream(gate_io_url: &str, tx: mpsc::Sender<StreamMessage>) {
    let (ws_stream, _) = connect_async(Url::parse(gate_io_url).unwrap())
        .await
        .unwrap();
    let (mut write, mut read) = ws_stream.split();
    let start = SystemTime::now();
    let since_the_epoch = start.duration_since(UNIX_EPOCH).unwrap();
    let current_time = since_the_epoch.as_secs();

    // Creating the subscription json message
    let subscribe_message = json!({
        "time": current_time,
        "channel": "spot.book_ticker",
        "event": "subscribe",
        "payload": ["POPCAT_USDT"]
    });
    let subscribe_message_2 = json!({
        "time": current_time,
        "channel": "spot.tickers",
        "event": "subscribe",
        "payload": ["SOL_USDT"]
    });

    let message_text = subscribe_message.to_string();
    let message_text2 = subscribe_message_2.to_string();
    write.send(Message::Text(message_text)).await.unwrap();
    write.send(Message::Text(message_text2)).await.unwrap();

    println!("Subscribed to the order book");
    let mut sol_price: f64 = 0.0;

    while let Some(Ok(Message::Text(text))) = read.next().await {
        if let Ok(json) = serde_json::from_str::<Value>(&text) {
            if !json["conn_id"].is_null() {
                //Skip the initial ws subscription success messages
                continue;
            }
            // update only the sol/usd price here, maybe you can notify the arbitrage checker process whenever sol price changes in the futures
            if json["channel"] == "spot.tickers" {
                sol_price = json["result"]["last"]
                    .as_str()
                    .unwrap()
                    .parse::<f64>()
                    .unwrap();
                continue;
            }
            //Updates for popcat/usd
            let ask_price = &json["result"]["a"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap();
            let bid_price = &json["result"]["b"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap();

            let _ = tx
                .send(StreamMessage::CexPrice(
                    bid_price.clone(),
                    ask_price.clone(),
                    sol_price,
                ))
                .await;
        }
    }
}
async fn process_messages(mut rx: mpsc::Receiver<StreamMessage>, token0: Pubkey, token1: Pubkey) {
    let mut raydium_pool = RaydiumPool {
        token0_amount: 0.0,
        token1_amount: 0.0,
        last_modified_slot: 0,
    };

    //Take the spread into account
    let mut cex_price_buy: f64 = 0.0;
    let mut cex_price_sell: f64 = 0.0;

    let mut sol_price: f64 = 0.0;

    while let Some(msg) = rx.recv().await {
        match msg {
            StreamMessage::Solana(addr, amount, slot) => {
                if addr == token0 {
                    raydium_pool.token0_amount = amount;

                    //Im not fully sure if there is a case for only one of the reserves changed, so make sure they both updated in the same slot.
                    if raydium_pool.last_modified_slot != slot {
                        raydium_pool.last_modified_slot = slot
                    } else {
                        check_arbitrage_opp(&raydium_pool, cex_price_buy, cex_price_sell, sol_price)
                    }
                } else if addr == token1 {
                    raydium_pool.token1_amount = amount;
                    if raydium_pool.last_modified_slot != slot {
                        raydium_pool.last_modified_slot = slot
                    } else {
                        check_arbitrage_opp(&raydium_pool, cex_price_buy, cex_price_sell, sol_price)
                    }
                } else {
                    unreachable!()
                }
            }
            StreamMessage::CexPrice(bid_price, ask_price, _sol_price) => {
                cex_price_buy = bid_price;
                cex_price_sell = ask_price;
                sol_price = _sol_price;
                check_arbitrage_opp(&raydium_pool, cex_price_buy, cex_price_sell, sol_price)
            }
        }
    }
}

fn check_arbitrage_opp(
    ray_pool: &RaydiumPool,
    cex_price_buy: f64,
    cex_price_sell: f64,
    sol_price: f64,
) {
    if ray_pool.token0_amount == 0.0 {
        return;
    }
    let dex_price_sol: f64 = ray_pool.token1_amount / ray_pool.token0_amount;
    let dex_price = dex_price_sol * sol_price;
    println!(
        "Dex price is: {}, cex price is: {}",
        dex_price, cex_price_buy
    );
    if (dex_price - cex_price_buy) / cex_price_buy * 10000.0 > MIN_BPS {
        println!("Arbitrage found CEX -> DEX");
    } else if (cex_price_sell - dex_price) / cex_price_sell * 10000.0 > MIN_BPS {
        println!("Arbitrage found DEX -> CEX");
    }
}

#[tokio::main]
async fn main() {
    let solana_url = "wss://mainnet.helius-rpc.com/?api-key=115eda22-77db-42e6-916c-9f53b21a5538";
    let gate_io_url = "wss://api.gateio.ws/ws/v4/"; // Replace with your actual WebSocket URL
    let vault1_addr: Pubkey = "4Vc6N76UBu26c3jJDKBAbvSD7zPLuQWStBk7QgVEoeoS" //token0
        .parse()
        .unwrap();
    let vault2_addr: Pubkey = "n6CwMY77wdEftf2VF6uPvbusYoraYUci3nYBPqH1DJ5" //token1
        .parse()
        .unwrap();

    let vault_addrs = vec![vault1_addr, vault2_addr]; //Raydium vault addresses for token0 and token1

    let (tx, rx) = mpsc::channel(100);

    //Listen dex updates
    tokio::spawn(handle_solana_streams(
        vault_addrs.clone(),
        solana_url,
        tx.clone(),
    ));

    //Listen cex updates
    tokio::spawn(handle_cex_stream(gate_io_url, tx));

    process_messages(rx, vault1_addr, vault2_addr).await;
}
