use tokio::sync::mpsc;

const MIN_BPS: f64 = 40.0; //Adjust your price difference in bps according to the fees

#[derive(Debug)]
pub enum StreamMessage {
    DexPrice(f64),
    CexPrice(f64, f64),
    SolPriceUpdate(f64),
}
pub struct CexPrice {
    bid_price: f64,
    ask_price: f64,
}
pub struct Strategy;
impl Strategy {
    pub fn new() -> Self {
        Self {}
    }
    pub async fn run(&self, mut rx: mpsc::Receiver<StreamMessage>) {
        //Take the spread into account
        let mut cached_cex_price = CexPrice {
            ask_price: 0.0,
            bid_price: 0.0,
        };
        let mut cached_sol_price: f64 = 0.0;
        let mut cached_dex_price: f64 = 0.0;

        while let Some(msg) = rx.recv().await {
            match msg {
                StreamMessage::DexPrice(_dex_price) => {
                    cached_dex_price = _dex_price;
                    self.check_arbitrage_opp(_dex_price, &cached_cex_price, cached_sol_price);
                }
                StreamMessage::CexPrice(bid_price, ask_price) => {
                    cached_cex_price.ask_price = ask_price;
                    cached_cex_price.bid_price = bid_price;
                    self.check_arbitrage_opp(cached_dex_price, &cached_cex_price, cached_sol_price)
                }
                StreamMessage::SolPriceUpdate(sol_price) => {
                    cached_sol_price = sol_price;
                    self.check_arbitrage_opp(cached_dex_price, &cached_cex_price, cached_sol_price)
                }
            }
        }
    }

    fn check_arbitrage_opp(&self, dex_price: f64, cex_price: &CexPrice, sol_price: f64) {
        if dex_price == 0.0 || cex_price.ask_price == 0.0 || sol_price == 0.0 {
            return;
        }
        let dex_price = dex_price * sol_price;
        if (dex_price - cex_price.bid_price) / cex_price.bid_price * 10000.0 > MIN_BPS {
            println!(
                "Arbitrage found CEX -> DEX, dex_price {}, cex_price {}, sol_price: {}",
                dex_price, cex_price.bid_price, sol_price,
            );
        } else if (cex_price.ask_price - dex_price) / cex_price.ask_price * 10000.0 > MIN_BPS {
            println!(
                "Arbitrage found DEX -> CEX dex_price {}, cex_price {}, sol_price:{}",
                dex_price, cex_price.ask_price, sol_price
            );
        }
    }
}
