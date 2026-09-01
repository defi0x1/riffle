use std::collections::HashSet;

use eyre::WrapErr;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{RetryTransientMiddleware, policies::ExponentialBackoff};
use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;

use crate::{FlowMetrics, RpcConfig, WindowedMetric};

use super::cache::NegativeCache;

pub struct DatapiClient {
    http: ClientWithMiddleware,
    base_url: String,
    page_size: u32,
    negative_cache: NegativeCache,
}

impl DatapiClient {
    pub fn new(config: &RpcConfig) -> eyre::Result<Self> {
        let retry_policy =
            ExponentialBackoff::builder().build_with_max_retries(config.max_retries as u32);
        let inner = reqwest::Client::builder()
            .build()
            .wrap_err_with(|| "Building datapi HTTP client")?;
        let http = ClientBuilder::new(inner)
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        Ok(Self {
            http,
            base_url: config.datapi_url.trim_end_matches('/').to_string(),
            page_size: config.datapi_page_size,
            negative_cache: NegativeCache::new(config.negative_cache_ttl),
        })
    }

    pub async fn flow_metrics(&self, pools: &[Pubkey]) -> eyre::Result<Vec<FlowMetrics>> {
        if pools.is_empty() {
            return Ok(Vec::new());
        }

        // There is no per-pool filter on the list endpoint, so a page walk is the only way
        // to answer any query. If every requested pool is a confirmed recent miss, skip it.
        if pools.iter().all(|pool| self.negative_cache.is_miss(pool)) {
            return Ok(Vec::new());
        }

        let universe = self.fetch_universe().await?;

        let wanted: HashSet<Pubkey> = pools.iter().copied().collect();
        let found: Vec<FlowMetrics> = universe
            .into_iter()
            .filter(|m| wanted.contains(&m.pool))
            .collect();

        let found_pools: HashSet<Pubkey> = found.iter().map(|m| m.pool).collect();
        for pool in pools {
            if !found_pools.contains(pool) {
                self.negative_cache.record_miss(*pool);
            }
        }

        Ok(found)
    }

    async fn fetch_universe(&self) -> eyre::Result<Vec<FlowMetrics>> {
        let mut page = 1u32;
        let mut out = Vec::new();

        loop {
            let url = format!(
                "{}/pools?page_size={}&page={page}",
                self.base_url, self.page_size
            );
            let response: DatapiPoolsResponse = self
                .http
                .get(&url)
                .send()
                .await
                .wrap_err_with(|| format!("Fetching datapi pools page {page}"))?
                .json()
                .await
                .wrap_err_with(|| format!("Parsing datapi pools page {page}"))?;

            let pages = response.pages.max(1);
            out.extend(
                response
                    .data
                    .into_iter()
                    .filter_map(|raw| match raw.into_flow_metrics() {
                        Ok(m) => Some(m),
                        Err(e) => {
                            tracing::warn!(error = ?e, "Skipping unparseable datapi pool row");
                            None
                        }
                    }),
            );

            if page >= pages {
                break;
            }
            page += 1;
        }

        Ok(out)
    }
}

#[derive(Deserialize)]
struct DatapiPoolsResponse {
    pages: u32,
    data: Vec<DatapiPool>,
}

#[derive(Deserialize)]
struct DatapiPool {
    address: String,
    #[serde(default)]
    current_price: f64,
    #[serde(default)]
    tvl: f64,
    #[serde(default)]
    apr: f64,
    #[serde(default)]
    apy: f64,
    #[serde(default)]
    dynamic_fee_pct: f64,
    #[serde(default)]
    pool_config: DatapiPoolConfig,
    #[serde(default)]
    volume: DatapiWindowed,
    #[serde(default)]
    fees: DatapiWindowed,
    #[serde(default)]
    fee_tvl_ratio: DatapiWindowed,
    #[serde(default)]
    has_farm: bool,
    #[serde(default)]
    is_blacklisted: bool,
    #[serde(default)]
    launchpad: Option<String>,
}

#[derive(Deserialize, Default)]
struct DatapiPoolConfig {
    #[serde(default)]
    bin_step: u16,
    #[serde(default)]
    base_fee_pct: f64,
    #[serde(default)]
    protocol_fee_pct: f64,
}

#[derive(Deserialize, Default)]
struct DatapiWindowed {
    #[serde(rename = "30m", default)]
    m30: f64,
    #[serde(rename = "1h", default)]
    h1: f64,
    #[serde(rename = "2h", default)]
    h2: f64,
    #[serde(rename = "4h", default)]
    h4: f64,
    #[serde(rename = "12h", default)]
    h12: f64,
    #[serde(rename = "24h", default)]
    h24: f64,
}

impl From<DatapiWindowed> for WindowedMetric {
    fn from(w: DatapiWindowed) -> Self {
        Self {
            m30: w.m30,
            h1: w.h1,
            h2: w.h2,
            h4: w.h4,
            h12: w.h12,
            h24: w.h24,
        }
    }
}

impl DatapiPool {
    fn into_flow_metrics(self) -> eyre::Result<FlowMetrics> {
        let pool: Pubkey = self
            .address
            .parse()
            .wrap_err_with(|| "Parsing pool address from datapi")?;
        Ok(FlowMetrics {
            pool,
            tvl: self.tvl,
            current_price: self.current_price,
            bin_step: self.pool_config.bin_step,
            base_fee_pct: self.pool_config.base_fee_pct,
            dynamic_fee_pct: self.dynamic_fee_pct,
            protocol_fee_pct: self.pool_config.protocol_fee_pct,
            apr: self.apr,
            apy: self.apy,
            volume: self.volume.into(),
            fees: self.fees.into(),
            fee_tvl_ratio: self.fee_tvl_ratio.into(),
            has_farm: self.has_farm,
            is_blacklisted: self.is_blacklisted,
            launchpad: self.launchpad,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_row_parses_windowed_fields_keyed_by_timeframe() {
        let raw = format!(
            r#"{{
            "address": "{}",
            "tvl": 12345.6,
            "current_price": 1.23,
            "apr": 4.5,
            "apy": 6.7,
            "dynamic_fee_pct": 0.02,
            "pool_config": {{ "bin_step": 25, "base_fee_pct": 0.1, "protocol_fee_pct": 5.0 }},
            "volume": {{ "30m": 1.0, "1h": 2.0, "2h": 3.0, "4h": 4.0, "12h": 5.0, "24h": 6.0 }},
            "fees": {{ "30m": 0.1, "1h": 0.2, "2h": 0.3, "4h": 0.4, "12h": 0.5, "24h": 0.6 }},
            "fee_tvl_ratio": {{ "30m": 0.01, "1h": 0.02, "2h": 0.03, "4h": 0.04, "12h": 0.05, "24h": 0.06 }},
            "has_farm": true,
            "is_blacklisted": false,
            "launchpad": null
        }}"#,
            Pubkey::new_unique()
        );

        let pool: DatapiPool = serde_json::from_str(&raw).unwrap();
        let metrics = pool.into_flow_metrics().unwrap();

        assert_eq!(metrics.bin_step, 25);
        assert_eq!(metrics.protocol_fee_pct, 5.0);
        assert_eq!(metrics.volume.h24, 6.0);
        assert_eq!(metrics.fees.m30, 0.1);
        assert!(metrics.has_farm);
        assert!(metrics.launchpad.is_none());
    }

    #[test]
    fn test_missing_optional_fields_default_rather_than_fail() {
        let raw = format!(
            r#"{{
            "address": "{}",
            "pool_config": {{ "bin_step": 10 }}
        }}"#,
            Pubkey::new_unique()
        );

        let pool: DatapiPool = serde_json::from_str(&raw).unwrap();
        let metrics = pool.into_flow_metrics().unwrap();
        assert_eq!(metrics.tvl, 0.0);
        assert_eq!(metrics.volume.h24, 0.0);
    }
}
