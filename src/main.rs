use std::collections::HashMap;

use argh::FromArgs;
use ethers::providers::{Middleware, Provider, ProviderError};
use serde::Deserialize;
use tokio::io;
use tokio::io::AsyncBufReadExt;
use tokio::sync::broadcast;
use tokio::task::JoinSet;

fn default_count() -> usize {
    100
}

#[derive(Debug, FromArgs)]
/// Send the same query to multiple rpcs and compare responses
struct VersusConfig {
    #[argh(positional, greedy)]
    rpcs: Vec<String>,

    /// how many rpc calls to test
    #[argh(option, default = "default_count()")]
    count: usize,
}

#[derive(Clone, Deserialize, Debug)]
struct JsonRpcRequest {
    method: String,
    params: Option<serde_json::Value>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Hello, world!");

    let config: VersusConfig = argh::from_env();

    let mut expected_chain_id = None;

    let mut http_providers = vec![];

    for rpc in config.rpcs.iter() {
        match Provider::try_from(rpc) {
            Ok(provider) => {
                let chain_id = provider.get_chainid().await?;

                println!("{}: chain_id {:#?}", rpc, chain_id);

                if let Some(expected_chain_id) = expected_chain_id {
                    if expected_chain_id != chain_id {
                        println!(
                            "{} has unexpected chain_id: {} != {}",
                            rpc, chain_id, expected_chain_id
                        );
                        continue;
                    }
                } else {
                    expected_chain_id = Some(chain_id);
                }

                http_providers.push(provider);
            }
            Err(err) => {
                println!("Failed connecting to {}: {:#?}", rpc, err);
                continue;
            }
        }
    }

    let num_providers = http_providers.len();

    if num_providers < 2 {
        println!("need at least 2 providers");
    }

    let (tx, _) = broadcast::channel::<JsonRpcRequest>(128);

    let mut set = JoinSet::new();

    for p in http_providers {
        let mut rx = tx.subscribe();

        let mut results = Vec::with_capacity(config.count);

        set.spawn(async move {
            while let Ok(data) = rx.recv().await {
                // println!("{} {}: {:?}", p.url(), i, data);

                let response: Result<serde_json::Value, ProviderError> =
                    p.request(&data.method, &data.params).await;

                results.push((data, response));

                // TODO: save this response somewhere so that we can compare to other rpcs
            }

            (p, results)
        });
    }

    // read jsonrpc lines from stdin and send to all the providers
    let stdin = io::stdin();

    let reader = io::BufReader::new(stdin);

    let mut count = 0;
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        let data: JsonRpcRequest = serde_json::from_str(&line)?;

        tx.send(data).expect("unable to send");

        count += 1;
        if count >= config.count {
            break;
        }
    }

    drop(tx);

    let mut map = HashMap::with_capacity(num_providers);

    while let Some(res) = set.join_next().await {
        match res {
            Ok((p, results)) => {
                map.insert(p.url().to_string(), results);
            }
            Err(err) => println!("error! {:#?}", err),
        };
    }

    println!("Goodbye, world!");
    Ok(())
}
