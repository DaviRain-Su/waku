//! Live native-token balances. RPC only; nothing is written to disk.

use std::time::Duration;

use alloy::primitives::Address;
use alloy::providers::{Provider, ProviderBuilder};
use anyhow::anyhow;
use proofship_protocol::web3::{
    EvmNetwork, WalletAccount, WalletBalance, WalletBalanceSnapshot, format_wei,
};

const RPC_TIMEOUT: Duration = Duration::from_secs(8);

pub fn fetch(
    wallets: &[WalletAccount],
    networks: &[EvmNetwork],
    wallet_id: Option<&str>,
) -> Vec<WalletBalanceSnapshot> {
    let wallets: Vec<&WalletAccount> = wallets
        .iter()
        .filter(|wallet| wallet_id.is_none_or(|id| wallet.id == id))
        .filter(|wallet| !wallet.address.trim().is_empty())
        .collect();
    let networks: Vec<&EvmNetwork> = networks.iter().filter(|network| network.enabled).collect();
    if wallets.is_empty() {
        return Vec::new();
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            return wallets
                .into_iter()
                .map(|wallet| WalletBalanceSnapshot {
                    wallet_id: wallet.id.clone(),
                    address: wallet.address.clone(),
                    balances: networks
                        .iter()
                        .map(|network| failed_balance(network, "could not start balance runtime"))
                        .collect(),
                })
                .collect();
        }
    };
    runtime.block_on(async {
        let mut snapshots = Vec::with_capacity(wallets.len());
        for wallet in wallets {
            let mut balances = Vec::with_capacity(networks.len());
            for network in &networks {
                balances.push(fetch_one(&wallet.address, network).await);
            }
            snapshots.push(WalletBalanceSnapshot {
                wallet_id: wallet.id.clone(),
                address: wallet.address.clone(),
                balances,
            });
        }
        snapshots
    })
}

async fn fetch_one(address: &str, network: &EvmNetwork) -> WalletBalance {
    match fetch_wei(address, &network.rpc_url).await {
        Ok(wei) => WalletBalance {
            network_id: network.id.clone(),
            network_name: network.name.clone(),
            symbol: network.currency_symbol.clone(),
            display: format_wei(&wei),
            wei,
            error: None,
        },
        Err(error) => failed_balance(network, &error.to_string()),
    }
}

fn failed_balance(network: &EvmNetwork, error: &str) -> WalletBalance {
    WalletBalance {
        network_id: network.id.clone(),
        network_name: network.name.clone(),
        symbol: network.currency_symbol.clone(),
        wei: "0".into(),
        display: "—".into(),
        error: Some(error.to_string()),
    }
}

async fn fetch_wei(address: &str, rpc_url: &str) -> anyhow::Result<String> {
    let parsed: Address = address
        .parse()
        .map_err(|error| anyhow!("invalid address: {error}"))?;
    let mut last_error = anyhow!("no RPC URL");
    let fallbacks: &[&str] = if rpc_url.contains("xlayer") {
        &[
            rpc_url,
            "https://testrpc.xlayer.tech/terigon",
            "https://xlayertestrpc.okx.com/terigon",
        ]
    } else {
        &[rpc_url]
    };
    for rpc_url in fallbacks {
        let Ok(url) = rpc_url.parse::<reqwest::Url>() else {
            continue;
        };
        let provider = ProviderBuilder::new().connect_http(url);
        match tokio::time::timeout(RPC_TIMEOUT, provider.get_balance(parsed)).await {
            Ok(Ok(balance)) => return Ok(balance.to_string()),
            Ok(Err(error)) => last_error = anyhow!("{error}"),
            Err(_) => last_error = anyhow!("RPC timed out ({rpc_url})"),
        }
    }
    Err(last_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proofship_protocol::web3::WalletSource;

    #[test]
    fn skips_rows_without_an_address() {
        let wallets = [WalletAccount {
            id: "env-1".into(),
            label: "dev".into(),
            address: String::new(),
            source: WalletSource::DevEnvKey,
            env_key_name: Some("DEV_PRIVATE_KEY".into()),
        }];
        let snapshots = fetch(&wallets, &[], None);
        assert!(snapshots.is_empty());
    }

    #[test]
    fn filters_by_wallet_id() {
        let wallets = [
            WalletAccount {
                id: "a".into(),
                label: "a".into(),
                address: "0x0000000000000000000000000000000000000001".into(),
                source: WalletSource::Watch,
                env_key_name: None,
            },
            WalletAccount {
                id: "b".into(),
                label: "b".into(),
                address: "0x0000000000000000000000000000000000000002".into(),
                source: WalletSource::Watch,
                env_key_name: None,
            },
        ];
        let snapshots = fetch(&wallets, &[], Some("b"));
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].wallet_id, "b");
        assert!(snapshots[0].balances.is_empty());
    }
}
