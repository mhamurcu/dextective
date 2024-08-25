use futures::{SinkExt, StreamExt};
use serde::{de::Error, Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

use crate::strategy::StreamMessage;

fn float_as_string<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer)?
        .parse()
        .map_err(serde::de::Error::custom)
}
#[derive(Debug, Deserialize)]
pub struct Ticker {
    #[serde(deserialize_with = "float_as_string")]
    pub last: f64,
}

#[derive(Debug)]
pub enum WebsocketEvent {
    OrderBook(OrderBook),
    Ticker(Ticker),
    SubscriptionResponse(Result<(), String>),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, PartialOrd)]
pub struct OrderBook {
    #[serde(rename = "b", deserialize_with = "float_as_string")]
    pub best_bid: f64,
    #[serde(rename = "a", deserialize_with = "float_as_string")]
    pub best_ask: f64,
}

pub struct GatePublicWebsocketClient<'a> {
    url: &'a str,
    sender: mpsc::Sender<StreamMessage>,
}

impl GatePublicWebsocketClient<'_> {
    pub fn new(sender: mpsc::Sender<StreamMessage>) -> Self {
        let gate_io_url = "wss://api.gateio.ws/ws/v4/";

        GatePublicWebsocketClient {
            url: gate_io_url,
            sender,
        }
    }
    pub async fn connect_and_subscribe(&self) {
        let (ws_stream, _) = connect_async(Url::parse(self.url).unwrap()).await.unwrap();
        let (mut write, read) = ws_stream.split();
        let start = SystemTime::now();
        let since_the_epoch = start.duration_since(UNIX_EPOCH).unwrap();
        let current_time = since_the_epoch.as_secs();

        let subscribe_message_ob = json!({
            "time": current_time,
            "channel": "spot.book_ticker",
            "event": "subscribe",
            "payload": ["POPCAT_USDT"]
        })
        .to_string();
        let subscribe_message_tickers = json!({
            "time": current_time,
            "channel": "spot.tickers",
            "event": "subscribe",
            "payload": ["SOL_USDT"]
        })
        .to_string();
        write
            .send(Message::Text(subscribe_message_ob))
            .await
            .unwrap();
        write
            .send(Message::Text(subscribe_message_tickers))
            .await
            .unwrap();

        let mut read = read.map(|message| match message {
            Ok(Message::Text(text)) => self.parse_message(&text),
            _ => Ok(None),
        });
        while let Some(event) = read.next().await {
            match event {
                Ok(Some(WebsocketEvent::OrderBook(order_book))) => {
                    let _ = self
                        .sender
                        .send(StreamMessage::CexPrice(
                            order_book.best_bid,
                            order_book.best_ask,
                        ))
                        .await;
                }
                Ok(Some(WebsocketEvent::Ticker(ticker))) => {
                    let _ = self
                        .sender
                        .send(StreamMessage::SolPriceUpdate(ticker.last))
                        .await;
                }
                Ok(Some(WebsocketEvent::SubscriptionResponse(response))) => {
                    println!("Received Subscription Response: {:?}", response);
                }
                Ok(None) => {
                    println!("Not specified event");
                }
                Err(e) => {
                    println!("Error receiving event: {:?}", e);
                }
            }
        }
    }
    fn parse_message(&self, msg: &str) -> Result<Option<WebsocketEvent>, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_str(msg)?;

        if let Some(event_type) = value.get("event") {
            match event_type.as_str() {
                Some("subscribe") => {
                    let subscription_result: String =
                        serde_json::from_value(value["result"]["status"].clone()).unwrap();
                    if subscription_result == "success" {
                        Ok(Some(WebsocketEvent::SubscriptionResponse(Ok(()))))
                    } else {
                        Ok(Some(WebsocketEvent::SubscriptionResponse(Err(
                            "Subscription failed".to_string(),
                        ))))
                    }
                }
                Some("update") => {
                    if let Some(channel_type) = value.get("channel") {
                        match channel_type.as_str() {
                            Some("spot.book_ticker") => {
                                let orderbook: OrderBook =
                                    serde_json::from_value(value["result"].clone())?;
                                Ok(Some(WebsocketEvent::OrderBook(orderbook)))
                            }
                            Some("spot.tickers") => {
                                let ticker: Ticker =
                                    serde_json::from_value(value["result"].clone())?;
                                Ok(Some(WebsocketEvent::Ticker(ticker)))
                            }
                            _ => Ok(None),
                        }
                    } else {
                        Err(serde_json::Error::custom("Expected update field"))
                    }
                }
                _ => Ok(None),
            }
        } else {
            Ok(None)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_gate() {
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            let gate_client = GatePublicWebsocketClient::new(tx.clone());
            gate_client.connect_and_subscribe().await;
        });
    }
}
