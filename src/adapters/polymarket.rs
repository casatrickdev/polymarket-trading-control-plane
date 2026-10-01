//! Read-only Polymarket adapter.
//!
//! Uses `polymarket_client_sdk_v2` 0.7 (CLOB v2, Data API, and the CLOB
//! WebSocket types). This module normalizes those types into control-plane
//! events. It does not place orders and it does not read private keys.
//! Authenticated order and trade snapshots are normalized from SDK responses
//! the caller already fetched with their own authenticated client.

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use polymarket_client_sdk_v2::clob::types::request::{MidpointRequest, OrderBookSummaryRequest};
use polymarket_client_sdk_v2::clob::types::response::{
    OpenOrderResponse, OrderBookSummaryResponse, TradeResponse,
};
use polymarket_client_sdk_v2::clob::types::{OrderStatusType, Side};
use polymarket_client_sdk_v2::clob::ws::types::response::{
    BookUpdate, OrderMessage, TradeMessage, WsMessage,
};
use polymarket_client_sdk_v2::clob::{Client as ClobClient, Config};
use polymarket_client_sdk_v2::data::types::request::PositionsRequest;
use polymarket_client_sdk_v2::data::types::request::TradesRequest;
use polymarket_client_sdk_v2::data::types::response::Position as SdkPosition;
use polymarket_client_sdk_v2::data::types::response::Trade as DataTrade;
use polymarket_client_sdk_v2::data::Client as DataClient;
use polymarket_client_sdk_v2::types::{Address, U256};
use rust_decimal::Decimal;

use super::{MarketDataSource, PositionSource, TradeStateSource};
use crate::domain::{
    BookSide, DomainEvent, FillRecord, MarketDataEvent, OrderEvent, OrderLifecycle, OrderRecord,
    OrderSide, PositionRecord,
};
use crate::error::ControlError;

pub const DEFAULT_CLOB_HOST: &str = "https://clob-v2.polymarket.com";
pub const DEFAULT_DATA_HOST: &str = "https://data-api.polymarket.com";

/// Public CLOB and Data API clients. No signer is stored here.
pub struct PolymarketReadClient {
    clob: ClobClient,
    data: DataClient,
}

impl PolymarketReadClient {
    pub fn new(clob_host: &str, data_host: &str) -> Result<Self, ControlError> {
        let clob = ClobClient::new(clob_host, Config::default())
            .map_err(|err| ControlError::Source(err.to_string()))?;
        let data =
            DataClient::new(data_host).map_err(|err| ControlError::Source(err.to_string()))?;
        Ok(Self { clob, data })
    }

    pub fn production() -> Result<Self, ControlError> {
        Self::new(DEFAULT_CLOB_HOST, DEFAULT_DATA_HOST)
    }

    pub async fn server_ok(&self) -> Result<String, ControlError> {
        self.clob
            .ok()
            .await
            .map(|status| status.to_string())
            .map_err(|err| ControlError::Source(err.to_string()))
    }

    pub async fn midpoint(&self, token_id: &str) -> Result<Decimal, ControlError> {
        let request = MidpointRequest::builder()
            .token_id(parse_u256(token_id)?)
            .build();
        let response = self
            .clob
            .midpoint(&request)
            .await
            .map_err(|err| ControlError::Source(err.to_string()))?;
        Ok(response.mid)
    }

    pub async fn order_book_event(&self, token_id: &str) -> Result<DomainEvent, ControlError> {
        let request = OrderBookSummaryRequest::builder()
            .token_id(parse_u256(token_id)?)
            .build();
        let book = self
            .clob
            .order_book(&request)
            .await
            .map_err(|err| ControlError::Source(err.to_string()))?;
        Ok(market_event_from_summary(&book))
    }

    pub async fn positions(&self, user: &str) -> Result<Vec<PositionRecord>, ControlError> {
        let address = Address::from_str(user)
            .map_err(|err| ControlError::Source(format!("invalid address: {err}")))?;
        let request = PositionsRequest::builder().user(address).build();
        let positions = self
            .data
            .positions(&request)
            .await
            .map_err(|err| ControlError::Source(err.to_string()))?;
        let observed_at = Utc::now();
        Ok(positions
            .iter()
            .map(|position| position_from_sdk(position, observed_at))
            .collect())
    }

    /// Public Data API trades for an address. These are market prints, not the authenticated CLOB order book.
    pub async fn user_trades(&self, user: &str) -> Result<Vec<FillRecord>, ControlError> {
        let address = Address::from_str(user)
            .map_err(|err| ControlError::Source(format!("invalid address: {err}")))?;
        let request = TradesRequest::builder().user(address).build();
        let trades = self
            .data
            .trades(&request)
            .await
            .map_err(|err| ControlError::Source(err.to_string()))?;
        Ok(trades.iter().map(fill_from_data_trade).collect())
    }
}

pub fn events_from_ws(message: &WsMessage, observed_at: DateTime<Utc>) -> Vec<DomainEvent> {
    match message {
        WsMessage::Book(book) => vec![market_event_from_book(book, observed_at)],
        WsMessage::LastTradePrice(price) => {
            vec![DomainEvent::MarketData(MarketDataEvent {
                token_id: price.asset_id.to_string(),
                market_id: price.market.to_string(),
                at: timestamp_from_i64(price.timestamp).unwrap_or(observed_at),
                best_bid: None,
                best_ask: None,
                last_trade_price: Some(price.price),
                snapshot: false,
            })]
        }
        WsMessage::Order(order) => vec![DomainEvent::Order(OrderEvent {
            side_of_book: BookSide::Local,
            order: order_from_ws(order, observed_at),
        })],
        WsMessage::BestBidAsk(update) => vec![DomainEvent::MarketData(MarketDataEvent {
            token_id: update.asset_id.to_string(),
            market_id: update.market.to_string(),
            at: timestamp_from_i64(update.timestamp).unwrap_or(observed_at),
            best_bid: Some(update.best_bid),
            best_ask: Some(update.best_ask),
            last_trade_price: None,
            snapshot: false,
        })],
        WsMessage::Trade(trade) => vec![DomainEvent::Fill(crate::domain::FillEvent {
            side_of_book: BookSide::Local,
            fill: fill_from_ws(trade, observed_at),
        })],
        _ => Vec::new(),
    }
}

pub fn order_from_open_order(order: &OpenOrderResponse) -> OrderRecord {
    OrderRecord {
        id: order.id.clone(),
        market_id: order.market.to_string(),
        token_id: order.asset_id.to_string(),
        side: map_side(order.side),
        price: order.price,
        original_size: order.original_size,
        size_matched: order.size_matched,
        status: lifecycle_from_status(&order.status, order.size_matched, order.original_size),
        updated_at: order.created_at,
    }
}

pub fn fill_from_trade(trade: &TradeResponse) -> FillRecord {
    FillRecord {
        id: trade.id.clone(),
        order_id: Some(trade.taker_order_id.clone()),
        market_id: trade.market.to_string(),
        token_id: trade.asset_id.to_string(),
        side: map_side(trade.side),
        price: trade.price,
        size: trade.size,
        at: trade.match_time,
    }
}

pub fn position_from_sdk(position: &SdkPosition, observed_at: DateTime<Utc>) -> PositionRecord {
    PositionRecord {
        market_id: position.condition_id.to_string(),
        token_id: position.asset.to_string(),
        size: position.size,
        avg_price: Some(position.avg_price),
        updated_at: observed_at,
    }
}

fn market_event_from_summary(book: &OrderBookSummaryResponse) -> DomainEvent {
    let best_bid = book.bids.first().map(|level| level.price);
    let best_ask = book.asks.first().map(|level| level.price);
    DomainEvent::MarketData(MarketDataEvent {
        token_id: book.asset_id.to_string(),
        market_id: book.market.to_string(),
        at: book.timestamp,
        best_bid,
        best_ask,
        last_trade_price: book.last_trade_price,
        snapshot: true,
    })
}

fn market_event_from_book(book: &BookUpdate, observed_at: DateTime<Utc>) -> DomainEvent {
    let best_bid = book.bids.first().map(|level| level.price);
    let best_ask = book.asks.first().map(|level| level.price);
    DomainEvent::MarketData(MarketDataEvent {
        token_id: book.asset_id.to_string(),
        market_id: book.market.to_string(),
        at: timestamp_from_i64(book.timestamp).unwrap_or(observed_at),
        best_bid,
        best_ask,
        last_trade_price: None,
        snapshot: true,
    })
}

fn order_from_ws(order: &OrderMessage, observed_at: DateTime<Utc>) -> OrderRecord {
    let original = order.original_size.unwrap_or(Decimal::ZERO);
    let matched = order.size_matched.unwrap_or(Decimal::ZERO);
    OrderRecord {
        id: order.id.clone(),
        market_id: order.market.to_string(),
        token_id: order.asset_id.to_string(),
        side: map_side(order.side),
        price: order.price,
        original_size: original,
        size_matched: matched,
        status: order
            .status
            .as_ref()
            .map(|status| lifecycle_from_status(status, matched, original))
            .unwrap_or(OrderLifecycle::Unknown),
        updated_at: order
            .timestamp
            .and_then(timestamp_from_i64)
            .unwrap_or(observed_at),
    }
}

fn fill_from_ws(trade: &TradeMessage, observed_at: DateTime<Utc>) -> FillRecord {
    let at = trade
        .timestamp
        .or(trade.matchtime)
        .or(trade.last_update)
        .and_then(timestamp_from_i64)
        .unwrap_or(observed_at);
    FillRecord {
        id: trade.id.clone(),
        order_id: trade.taker_order_id.clone(),
        market_id: trade.market.to_string(),
        token_id: trade.asset_id.to_string(),
        side: map_side(trade.side),
        price: trade.price,
        size: trade.size,
        at,
    }
}

fn fill_from_data_trade(trade: &DataTrade) -> FillRecord {
    FillRecord {
        id: format!("{}:{}", trade.transaction_hash, trade.asset),
        order_id: None,
        market_id: trade.condition_id.to_string(),
        token_id: trade.asset.to_string(),
        side: map_data_side(&trade.side),
        price: trade.price,
        size: trade.size,
        at: timestamp_from_i64(trade.timestamp).unwrap_or_else(Utc::now),
    }
}

fn map_side(side: Side) -> OrderSide {
    match side {
        Side::Buy => OrderSide::Buy,
        Side::Sell => OrderSide::Sell,
        Side::Unknown => OrderSide::Unknown,
        _ => OrderSide::Unknown,
    }
}

fn map_data_side(side: &polymarket_client_sdk_v2::data::types::Side) -> OrderSide {
    use polymarket_client_sdk_v2::data::types::Side as DataSide;
    match side {
        DataSide::Buy => OrderSide::Buy,
        DataSide::Sell => OrderSide::Sell,
        DataSide::Unknown(_) => OrderSide::Unknown,
        _ => OrderSide::Unknown,
    }
}

fn lifecycle_from_status(
    status: &OrderStatusType,
    matched: Decimal,
    original: Decimal,
) -> OrderLifecycle {
    match status {
        OrderStatusType::Canceled => OrderLifecycle::Cancelled,
        OrderStatusType::Live | OrderStatusType::Delayed | OrderStatusType::Unmatched => {
            if matched > Decimal::ZERO && original > Decimal::ZERO && matched < original {
                OrderLifecycle::PartiallyFilled
            } else {
                OrderLifecycle::Open
            }
        }
        OrderStatusType::Matched => {
            if matched > Decimal::ZERO && original > Decimal::ZERO && matched < original {
                OrderLifecycle::PartiallyFilled
            } else {
                OrderLifecycle::Filled
            }
        }
        OrderStatusType::Unknown(_) => OrderLifecycle::Unknown,
        _ => OrderLifecycle::Unknown,
    }
}

fn parse_u256(token_id: &str) -> Result<U256, ControlError> {
    U256::from_str(token_id).map_err(|err| ControlError::Source(format!("invalid token id: {err}")))
}

#[async_trait]
impl MarketDataSource for PolymarketReadClient {
    async fn latest(&self, token_id: &str) -> Result<MarketDataEvent, ControlError> {
        match self.order_book_event(token_id).await? {
            DomainEvent::MarketData(event) => Ok(event),
            _ => Err(ControlError::Source(
                "order book response was not market data".into(),
            )),
        }
    }
}

/// Data API positions for one address. The address is public account identity, not a key.
pub struct PolymarketAccount {
    client: PolymarketReadClient,
    user: String,
}

impl PolymarketAccount {
    pub fn new(
        clob_host: &str,
        data_host: &str,
        user: impl Into<String>,
    ) -> Result<Self, ControlError> {
        Ok(Self {
            client: PolymarketReadClient::new(clob_host, data_host)?,
            user: user.into(),
        })
    }

    pub fn client(&self) -> &PolymarketReadClient {
        &self.client
    }
}

#[async_trait]
impl PositionSource for PolymarketAccount {
    async fn positions(&self) -> Result<Vec<PositionRecord>, ControlError> {
        self.client.positions(&self.user).await
    }
}

#[async_trait]
impl TradeStateSource for PolymarketAccount {
    async fn recent_trades(&self) -> Result<Vec<FillRecord>, ControlError> {
        self.client.user_trades(&self.user).await
    }
}

#[async_trait]
impl MarketDataSource for PolymarketAccount {
    async fn latest(&self, token_id: &str) -> Result<MarketDataEvent, ControlError> {
        self.client.latest(token_id).await
    }
}

fn timestamp_from_i64(raw: i64) -> Option<DateTime<Utc>> {
    let seconds = if raw.abs() > 10_000_000_000 {
        raw / 1000
    } else {
        raw
    };
    DateTime::from_timestamp(seconds, 0)
}
