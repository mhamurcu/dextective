# Dex Cex Arbitrage Monitor in Solana

 - POPCAT/SOL is selected from RaydiumV4 pool, since it looks easier to integrate constant product amm, and raydium v4 is supporting that.
 - In the CEX side, gate io spot cex is used.
 - Since CEX side pair is POPCAT/USDT, I need to listen for SOL/USDT price as well. And check for difference between POPCAT/SOL * SOL/USDT <=> POPCAT/USDT
 - For calculating DEX price, subscribed to the account changes for each of the token0 and token1 reserves, if they are both updated in the same slot, then we can say that price is moved on dex side.
 - For calculating CEX price, simply subscribed to the best bid and best ask prices in the orderbook. Capacity is currently disregarded.
 
 